#!/usr/bin/env python3
"""Isolated public-text retrieval spike; no application API or dependency changes."""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import platform
import re
import resource
import statistics
import time
import urllib.request


def tokens(text):
    return re.findall(r"[^\W_]+", text.lower(), re.UNICODE)


def current_chat_score(document, query):
    # Match artifacts.rs::score_chunk, including substring token matching.
    text, needle = document.lower(), " ".join(query.split()).lower()
    score = 4.0 if needle and needle in text else 0.0
    for token in tokens(query):
        if len(token) >= 3 and token in text:
            score += 1.0 + min(text.count(token), 6) * 0.15
    return score


def bm25(documents, query):
    counts = [Counter(tokens(document)) for document in documents]
    lengths = [sum(count.values()) for count in counts]
    average = sum(lengths) / len(lengths) or 1.0
    scores = [0.0] * len(counts)
    for token in set(tokens(query)):
        frequency = sum(token in count for count in counts)
        weight = math.log(1 + (len(counts) - frequency + 0.5) / (frequency + 0.5))
        for i, count in enumerate(counts):
            tf = count[token]
            if tf:
                scores[i] += weight * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * lengths[i] / average))
    return scores


def ranked(scores, positive_only=False):
    return sorted((i for i, value in enumerate(scores) if not positive_only or value > 0),
                  key=lambda i: (-scores[i], i))


def normalized(vector, dimension):
    if len(vector) < dimension or not all(math.isfinite(x) for x in vector):
        raise ValueError("invalid or undersized embedding")
    vector = vector[:dimension]
    norm = math.sqrt(sum(x * x for x in vector))
    if norm == 0:
        raise ValueError("zero embedding")
    return [x / norm for x in vector]


def evaluate(corpus, vectors=None, dimension=256, scope="archive"):
    docs, queries = corpus["documents"], corpus["queries"]
    ids = [doc["id"] for doc in docs]
    if not ids or len(set(ids)) != len(ids) or len({q["id"] for q in queries}) != len(queries):
        raise ValueError("empty corpus or duplicate IDs")
    texts = [doc["text"] for doc in docs]
    encoded = None
    if vectors is not None:
        if set(vectors["documents"]) != set(ids) or set(vectors["queries"]) != {q["id"] for q in queries}:
            raise ValueError("embedding IDs do not match frozen corpus")
        encoded = [normalized(vectors["documents"][key], dimension) for key in ids]
    rows = []
    for query in queries:
        relevant = set(query["relevant_doc_ids"])
        if not relevant <= set(ids):
            raise ValueError("unknown relevance ID")
        started = time.perf_counter()
        if scope == "recording" and not query.get("recording_id"):
            raise ValueError("recording scope requires query recording_id")
        eligible = [i for i, doc in enumerate(docs)
                    if scope == "archive" or doc.get("recording_id") == query.get("recording_id")]
        if not eligible:
            raise ValueError("query has no documents in its recording scope")
        def eligible_ranking(scores, positive_only=False):
            return [eligible[i] for i in ranked([scores[j] for j in eligible], positive_only)]
        needle = query["query"].strip().lower()
        exact = [float(bool(needle) and needle in text.lower()) for text in texts]
        lexical = [0.0] * len(docs)
        for i, score in zip(eligible, bm25([texts[j] for j in eligible], query["query"])):
            lexical[i] = score
        chat_scores = [current_chat_score(text, query["query"]) for text in texts]
        chat = eligible_ranking(chat_scores, True)[:10]
        if not chat:
            chat = eligible[:4]
        methods = {"history_substring": eligible_ranking(exact, True), "current_chat": chat,
                   "bm25": eligible_ranking(lexical, True)}
        semantic_scores = None
        if encoded is not None:
            qv = normalized(vectors["queries"][query["id"]], dimension)
            semantic_scores = [sum(x * y for x, y in zip(qv, dv)) for dv in encoded]
            methods["semantic"] = eligible_ranking(semantic_scores)
            # Fixed reciprocal-rank fusion; never tuned on these query labels.
            fusion = [0.0] * len(docs)
            for ordering in (methods["bm25"], methods["semantic"]):
                for position, index in enumerate(ordering, 1):
                    fusion[index] += 1.0 / (60 + position)
            methods["hybrid_rrf"] = eligible_ranking(fusion)
        for method, ordering in methods.items():
            retrieved = [ids[i] for i in ordering]
            positions = [position for position, key in enumerate(retrieved, 1) if key in relevant]
            support = set(query.get("support_doc_ids", relevant))
            if not support <= set(ids):
                raise ValueError("unknown answer-support ID")
            rows.append({"query_id": query["id"], "category": query["category"],
                         "split": query.get("split", "test"), "language": query.get("language"),
                         "query": query["query"], "method": method,
                         "answerable": query.get("answerable", bool(support)),
                         "support_doc_ids": sorted(support),
                         "support_hit1": bool(support.intersection(retrieved[:1])),
                         "support_hit5": bool(support.intersection(retrieved[:5])),
                         "relevant_doc_ids": sorted(relevant), "top5": retrieved[:5],
                         "first_relevant_rank": min(positions) if positions else None,
                         "hit1": bool(relevant.intersection(retrieved[:1])),
                         "hit5": bool(relevant.intersection(retrieved[:5])),
                         "reciprocal_rank": 1 / min(positions) if positions else 0,
                         "best_cosine": max(semantic_scores[i] for i in eligible) if semantic_scores else None,
                         "scoring_seconds_all_methods": time.perf_counter() - started})
    summary = {}
    for method in sorted({row["method"] for row in rows}):
        for category in ["all_positive"] + sorted({row["category"] for row in rows}):
            selected = [row for row in rows if row["method"] == method and row["relevant_doc_ids"]
                        and (category == "all_positive" or row["category"] == category)]
            if selected:
                summary[f"{method}/{category}"] = {
                    "queries": len(selected),
                    "hit1": sum(row["hit1"] for row in selected) / len(selected),
                    "hit5": sum(row["hit5"] for row in selected) / len(selected),
                    "mrr": sum(row["reciprocal_rank"] for row in selected) / len(selected)}
    support_summary = {}
    for method in sorted({row["method"] for row in rows}):
        for split in sorted({row["split"] for row in rows}):
            selected = [row for row in rows if row["method"] == method and row["split"] == split
                        and row["answerable"]]
            if selected:
                support_summary[f"{method}/{split}"] = {"queries": len(selected),
                    "support_hit1": sum(row["support_hit1"] for row in selected) / len(selected),
                    "support_hit5": sum(row["support_hit5"] for row in selected) / len(selected)}
    calibration = {}
    if encoded is not None:
        dev = [row for row in rows if row["method"] == "semantic" and row["split"] == "dev"]
        positives = [row for row in dev if row["answerable"]]
        negatives = [row for row in dev if not row["answerable"]]
        if positives and negatives:
            # Fixed policy: reject all dev negatives, then measure unchanged on test.
            threshold = math.nextafter(max(row["best_cosine"] for row in negatives), math.inf)
            for split in ("dev", "test"):
                selected = [row for row in rows if row["method"] == "semantic" and row["split"] == split]
                pos = [row for row in selected if row["answerable"]]
                neg = [row for row in selected if not row["answerable"]]
                calibration[split] = {"threshold": threshold, "positives": len(pos), "negatives": len(neg),
                    "accepted_positives": sum(row["best_cosine"] >= threshold for row in pos),
                    "false_accept_negatives": sum(row["best_cosine"] >= threshold for row in neg),
                    "accepted_with_gold_support_top5": sum(row["best_cosine"] >= threshold and row["support_hit5"] for row in pos)}
    return {"dimension": dimension if vectors else None, "scope": scope, "documents": len(docs),
            "queries": len(queries), "summary": summary, "rows": rows,
            "support_summary": support_summary, "calibration": calibration,
            "negative_policy": "Dev-only threshold rejects all dev negatives, without test tuning; not proof of answerability." if calibration else "No calibrated semantic abstention threshold. Negative rankings are diagnostic, not answers.",
            "limits": "Manually curated exploratory queries; "
                      + ("real public ASR output; " if corpus.get("provenance", {}).get("asr_run") else "reference text, not ASR output; ")
                      + ("timed passages from long recordings; not app chat execution." if corpus.get("long_recordings") else "independent short passages, not full recordings.")}


def self_test():
    assert tokens("M3 16 GB e 1,2") == ["m3", "16", "gb", "e", "1", "2"]
    assert current_chat_score("Budget budget", "budget") == 5.3
    assert abs(sum(x * x for x in normalized([3.0, 4.0, 9.0], 2)) - 1) < 1e-12
    for invalid in ([float("nan"), 1], [0, 0], [1]):
        try:
            normalized(invalid, 2)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid embedding accepted")
    corpus = {"documents": [{"id": "a", "text": "budget 2026"}, {"id": "b", "text": "mare"}],
              "queries": [{"id": "q", "query": "2026", "category": "numbers", "relevant_doc_ids": ["a"]}]}
    result = evaluate(corpus, {"documents": {"a": [1, 0], "b": [0, 1]}, "queries": {"q": [1, 0]}}, 2)
    assert all(row["hit1"] for row in result["rows"])
    corpus["documents"][0]["recording_id"] = "A"
    corpus["documents"][1]["recording_id"] = "B"
    corpus["queries"] = [
        {"id": key, "query": "budget", "category": "probe", "recording_id": "A",
         "split": split, "answerable": answerable, "support_doc_ids": ["a"] if answerable else [],
         "relevant_doc_ids": ["a"]}
        for key, split, answerable in [("dp", "dev", True), ("dn", "dev", False),
                                      ("tp", "test", True), ("tn", "test", False)]]
    vectors = {"documents": {"a": [1, 0], "b": [0, 1]},
               "queries": {"dp": [1, 0], "dn": [.8, .6], "tp": [0, 1], "tn": [1, 0]}}
    result = evaluate(corpus, vectors, 2, "recording")
    assert all(row["top5"] == ["a"] for row in result["rows"])
    assert result["calibration"]["dev"]["false_accept_negatives"] == 0
    assert result["calibration"]["test"]["false_accept_negatives"] == 1
    assert result["calibration"]["test"]["accepted_positives"] == 0
    print("Embedding spike checks passed")


def encode(corpus, args):
    """Load one model, warm it up, encode once, repeat queries three times."""
    import importlib.metadata
    loaded = time.perf_counter()
    if args.runtime == "litert":
        from litert_lm import Backend, EmbeddingEngine, EmbeddingOptions, InputOverflowStrategy
        backend = Backend.CPU(thread_count=4) if args.backend == "cpu" else Backend.GPU()
        engine = EmbeddingEngine(model_path=args.model, backend=backend)
        engine.__enter__()
        options = EmbeddingOptions(normalize=True, output_size=768, input_overflow_strategy=InputOverflowStrategy.ERROR)
        inference = lambda text: list(engine.compute_embedding(text, options).embedding)
        versions = {name: importlib.metadata.version(name) for name in ("litert-lm", "litert-lm-api")}
    elif args.runtime == "sentence-transformers":
        import torch
        from sentence_transformers import SentenceTransformer
        torch.set_num_threads(4)
        engine = SentenceTransformer(args.model, device="cpu", model_kwargs={"torch_dtype": torch.float32})
        inference = lambda text: engine.encode(text, normalize_embeddings=True).tolist()
        versions = {name: importlib.metadata.version(name) for name in ("sentence-transformers", "transformers", "torch")}
    else:
        def inference(text):
            request = urllib.request.Request(args.url.rstrip("/") + "/v1/embeddings",
                                             data=json.dumps({"input": text, "model": args.model}).encode(),
                                             headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=120) as response:
                return json.load(response)["data"][0]["embedding"]
        versions = {"runtime_version": args.runtime_version or "not recorded"}
    load_seconds = time.perf_counter() - loaded

    def formatted(text, is_query):
        if args.prefix == "hf":
            return ("task: search result | query: " if is_query else "title: none | text: ") + text
        if args.prefix == "litert":
            return ("task: search query | text: " if is_query else "task: search result | text: ") + text
        return text

    try:
        warm = time.perf_counter()
        normalized(inference(formatted("warmup", True)), 768)
        warm_seconds = time.perf_counter() - warm
        result = {"documents": {}, "queries": {}}
        document_seconds, query_seconds = [], []
        for doc in corpus["documents"]:
            started = time.perf_counter()
            result["documents"][doc["id"]] = inference(formatted(doc["text"], False))
            normalized(result["documents"][doc["id"]], 768)
            document_seconds.append(time.perf_counter() - started)
        determinism = True
        for repeat in range(3):
            for query in corpus["queries"]:
                started = time.perf_counter()
                vector = inference(formatted(query["query"], True))
                normalized(vector, 768)
                query_seconds.append(time.perf_counter() - started)
                if repeat == 0:
                    result["queries"][query["id"]] = vector
                else:
                    determinism &= max(abs(x - y) for x, y in zip(vector, result["queries"][query["id"]])) < 1e-5
        ordered = sorted(query_seconds)
        result["runtime"] = {"engine": args.runtime, "model": args.model, "backend": args.backend,
                             "platform": platform.platform(), "packages": versions, "prefix": args.prefix,
                             "load_seconds": load_seconds, "warmup_seconds": warm_seconds,
                             "document_total_seconds": sum(document_seconds), "document_seconds": document_seconds,
                             "query_seconds": query_seconds, "query_repeats": 3,
                             "query_median_seconds": statistics.median(query_seconds),
                             "query_p95_seconds": ordered[math.ceil(len(ordered) * .95) - 1],
                             "repeat_vectors_within_1e-5": determinism,
                             "process_peak_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * (1 if platform.system() == "Darwin" else 1024)}
        if args.runtime == "llama":
            result["runtime"]["memory_scope"] = "HTTP client only; native server memory and startup must be measured separately"
        return result
    finally:
        if args.runtime == "litert":
            engine.__exit__(None, None, None)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--vectors", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--dimension", type=int, choices=(128, 256, 512, 768), default=256)
    parser.add_argument("--scope", choices=("archive", "recording"), default="archive")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--runtime", choices=("litert", "sentence-transformers", "llama"))
    parser.add_argument("--model")
    parser.add_argument("--backend", choices=("cpu", "gpu"), default="cpu")
    parser.add_argument("--prefix", choices=("hf", "litert", "none"), default="hf")
    parser.add_argument("--url", default="http://127.0.0.1:9381")
    parser.add_argument("--runtime-version")
    parser.add_argument("--asr-run", type=Path, help="Freeze the same labels with public ASR output replacing reference documents")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.corpus or not args.output:
        parser.error("--corpus and --output required")
    data = args.corpus.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if args.asr_run:
        corpus = json.loads(data)
        samples = json.loads(args.asr_run.read_text())["samples"]
        by_id = {sample["sample_id"]: sample for sample in samples}
        for doc in corpus["documents"]:
            sample = by_id[doc["id"]]
            if sample["status"] != "ok" or not sample["text"].strip():
                raise ValueError("missing or failed ASR document")
            doc["reference_text_sha256"] = hashlib.sha256(doc["text"].encode()).hexdigest()
            doc["text"] = sample["text"]
        corpus["provenance"] = {"source_corpus_sha256": digest,
                                "asr_run": str(args.asr_run),
                                "asr_run_sha256": hashlib.sha256(args.asr_run.read_bytes()).hexdigest(),
                                "selection": "Representative document ID; unchanged relevance labels and query text"}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n")
        return
    if args.runtime:
        if not args.model:
            parser.error("--model required for encoding")
        if args.runtime != "litert" and args.backend != "cpu":
            parser.error("reference SentenceTransformers run is CPU only")
        result = encode(json.loads(data), args)
        result["corpus_sha256"] = digest
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, ensure_ascii=False) + "\n")
        print(json.dumps(result["runtime"], indent=2))
        return
    vectors = json.loads(args.vectors.read_text()) if args.vectors else None
    if vectors and vectors.get("corpus_sha256") != digest:
        raise ValueError("embedding corpus checksum mismatch")
    result = evaluate(json.loads(data), vectors, args.dimension, args.scope)
    result.update(corpus_sha256=digest, vectors_sha256=hashlib.sha256(args.vectors.read_bytes()).hexdigest() if args.vectors else None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
