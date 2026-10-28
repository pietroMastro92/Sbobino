# Companion AI v1 — development contract

This is an opt-in feature under development on canonical `main`, not a capability of the published v2.0.36 tag. The iOS companion repository is separate. No release or promotion is part of this implementation.

## Pairing and local execution

The QR contains exactly the existing version-1 pairing JSON (endpoint, bearer token, DER certificate in base64, desktop name/version). Generate it locally; never send it to a QR service or log the payload. The iPhone asks the user to confirm pairing, verifies the pinned certificate and authenticated health response, then stores the pairing securely. Both devices must be able to reach the advertised local network endpoint. This is not an internet relay or WhatsApp account service. Revocation stops the listener, drains remote work and invalidates the old token/certificate; a new session creates new credentials.

AI is disabled on each new Desktop session. The owner explicitly chooses a configured Apple Foundation or Ollama provider in Desktop settings. No global/cloud fallback is used. Ollama must use loopback HTTP without credentials, proxy, redirect or DNS resolution. Its local weights/context metadata is checked before probing and before each generation; cloud model metadata is rejected even when the endpoint is localhost. Foundation uses its own cancellable, bounded-output bridge process. AI and ASR share the existing heavy-work gate to avoid simultaneous model memory pressure.

Authenticated `GET /v1/health` contains `ai: { available, provider, model, unavailable_reason, limits }` and `asr_available`. Capability `companion_ai_v1` is advertised only after a successful local generation probe. AI can be ready while ASR is unavailable; in that case health returns 200 and ASR `engine`/`model` may be absent. Protocol support does not imply optional diarization assets are installed. No system network, VPN or firewall setting is changed.

Key points use an internal structured `points` array, validated before rendering a Markdown bullet list in the public `result.text`. Other operations use an internal `text` field. Ollama receives the operation's JSON schema; Foundation uses native guided generation for typed text/points and citations. Both providers pass through the same result/citation validation. A prose-only key-points response or duplicate citation keys is a failed job, never a successful substitute. Explanations request explicit source relationships and identify missing links instead of completing them from general knowledge. This does not change the phone wire format or certify model accuracy.

## Requests

`PUT /v1/ai/jobs/{UUID}`, JSON:

```json
{
  "version": 1,
  "operation": "summary",
  "provider": "ollama",
  "locale": "it",
  "prompt": null,
  "sources": [{
    "recording_id": "recording-uuid",
    "title": "Selected lesson",
    "revision": "SHA-256 of exact UTF-8 segment text joined with newline",
    "segments": [{"id": "segment-id", "text": "Selected source text", "start_seconds": 0, "end_seconds": 2}]
  }]
}
```

Operations: `summary`, `key_points`, `explain`, `question`, `revise`. Provider must match the explicitly selected Desktop backend (`foundation_apple` or `ollama`). Recording IDs are unique within the request, segment IDs within each recording. Order is significant. Unknown fields, invalid revisions, duplicate IDs, empty segments and invalid times are rejected. A revision is lowercase SHA-256 of `segments.map(text).joined(separator: "\n")`, without trimming or normalization.

Health limits: `max_request_bytes` = 524288, `max_sources` = 32, `max_segments` = 512, `max_prompt_characters` = 4000, `max_text_characters` up to 24000 for Ollama / 6000 for Foundation, and `max_prompt_bytes` for the fully composed model prompt. Text characters are Unicode scalar values, not bytes or grapheme clusters. The effective byte budget can be smaller for the selected model. No silent truncation, top-k source selection or incomplete-folder summary is allowed: clients split oversized inputs into explicitly tracked complete jobs. Ollama reserves output space and requests sufficient context using a conservative input-byte bound; unknown/too-small context is refused.

## State, recovery and acknowledgement

`GET /v1/ai/jobs/{UUID}` returns `{ id, status, phase, percentage, result?, error? }`. `error` is an optional human-readable string. A completed result is `{ text, citations: [{recording_id, segment_id}], provider, model }`. Citations must refer to supplied segments; summary/key-points results must cite every selected recording. ID validation is not a proof of factual entailment: AI output remains a reviewable proposal. Original transcripts and artifacts are never modified by these routes.

Statuses: `queued`, `running`, `completed`, `failed`, `cancelled`, `interrupted`, `acknowledged`. New PUT returns 202. Repeating the same UUID and typed request returns the existing state (200), without another inference. Reusing a UUID with a different request returns 409. A server restart marks pending jobs `interrupted`; GET and repeated PUT never resume them. An explicit retry uses a new UUID with the original saved sources. Source/request data is encrypted on disk before admission; result/state commits use atomic encrypted files. An exclusive store lease prevents overlapping sessions from overwriting recovery state.

`DELETE /v1/ai/jobs/{UUID}` cancels nonterminal work; it is not delivery acknowledgement. `POST /v1/ai/jobs/{UUID}/ack`, body `{"version":1}`, is an explicit, idempotent acknowledgement and returns 200/`acknowledged`. The iPhone must first save the result or preserve sources/new retry UUID durably. ACK is accepted for any terminal status, including explicit abandonment of failed/cancelled/interrupted work. It replaces the encrypted payload/result with a durable UUID/digest tombstone. The server cannot independently prove that the client saved data. Failed ACK persistence returns an error and retains the payload. Never auto-ACK on receipt or use DELETE as ACK.

No unacknowledged record is silently evicted. Capacity is 64 unacknowledged jobs and 4096 total tombstones/records; exhaustion returns 429, never reuse/erase old UUIDs silently. A storage failure remains visible; the last durable request is preserved even if the current process can only report a volatile storage error. Optional AI storage failure disables AI without disabling the ASR listener. Selection is not automatically restored after restart.

HTTP errors are `{ "error": { "code": "...", "message": "..." } }`: 400 `invalid_request`/`invalid_revision`, 404 `job_not_found`, 409 `job_conflict`/`provider_changed`/`job_not_terminal`, 413 `input_too_large`, 429 `queue_full`, 503 `ai_unavailable`, 500 `storage_unavailable`. A 404 is not permission to delete phone sources. Upload has a 30-second deadline; each provider call is bounded and the full queued job has a 600-second deadline. Failed work requires explicit retry, without a cloud fallback.

Legacy `/v1/jobs/{UUID}` audio transfer remains a separate in-memory protocol: a Desktop restart or terminal eviction can lose its transport state even though the saved Desktop artifact survives. The new AI ACK/durability semantics must not be attributed to that existing route.

## Verification

Use synthetic data and UltraDisk-backed profiles. Unit HTTP fixtures prove protocol/state behavior, not real model quality. The `companion_ai_smoke` example requires a new empty profile, explicit local provider and a private pairing-file path; `--resume` is accepted only for a marked smoke profile. Pairing credentials are never printed. Preserve hashes, commands and results separately for fixture, loopback real-provider and physical iPhone transport tests. Missing native-platform or physical transport evidence remains unverified.
