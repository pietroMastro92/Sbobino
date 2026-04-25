import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { supportedAppLanguages, translationsCatalog } from "./i18n";

const sourceDir = path.dirname(fileURLToPath(import.meta.url));

function walkSourceFiles(directory: string): string[] {
  const entries = fs.readdirSync(directory, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...walkSourceFiles(absolutePath));
      continue;
    }

    if (
      !entry.isFile()
      || (!absolutePath.endsWith(".ts") && !absolutePath.endsWith(".tsx"))
      || absolutePath.endsWith(".test.ts")
      || absolutePath.endsWith(".test.tsx")
      || absolutePath.endsWith(".d.ts")
      || absolutePath.endsWith(path.sep + "i18n.ts")
      // ErrorBoundary is the safety net that must render even if the i18n
      // catalog itself cannot load, so its copy is intentionally hardcoded
      // English. Skip it from the raw-visible-JSX gate.
      || absolutePath.endsWith(path.sep + "ErrorBoundary.tsx")
    ) {
      continue;
    }

    files.push(absolutePath);
  }

  return files;
}

function collectLiteralTranslationKeys(): string[] {
  const keyPattern = /\bt\(\s*["'`]([^"'`$]+)["'`]/g;
  const keys = new Set<string>();

  for (const filePath of walkSourceFiles(sourceDir)) {
    const source = fs.readFileSync(filePath, "utf8");
    for (const match of source.matchAll(keyPattern)) {
      keys.add(match[1]);
    }
  }

  return [...keys].sort();
}

function collectRawVisibleJsxText(): string[] {
  const files = walkSourceFiles(sourceDir).filter((filePath) => filePath.endsWith(".tsx"));
  const textNodePattern = /<([A-Za-z][\w.-]*)[^>]*>\s*([^<{]*[A-Za-z0-9][^<{]*)\s*<\/\1>/g;
  const attributePattern = /\b(placeholder|title|aria-label)\s*=\s*(["'])([^{"']*[A-Za-z][^"']*)\2/g;
  const allowedTextNodes = new Set([
    "CPU",
    "MPS",
    "0.75x",
    "1x",
    "1.25x",
    "1.5x",
    "1.75x",
    "2x",
    "`--threads`",
    "`--processors`",
    "`--temperature`",
    "`--entropy-thold`",
    "`--logprob-thold`",
    "`--word-thold`",
  ]);
  const allowedTextPatterns = [
    /^\d+$/,
    /^\d+(?:\.\d+)?x$/,
  ];
  const findings: string[] = [];

  for (const filePath of files) {
    const source = fs.readFileSync(filePath, "utf8");
    const lines = source.split(/\r?\n/);

    for (const [index, line] of lines.entries()) {
      if (!line.includes("<")) {
        continue;
      }

      for (const match of line.matchAll(textNodePattern)) {
        const rawText = match[2].trim();
        if (!rawText || rawText.includes("{") || rawText.includes("}")) {
          continue;
        }
        if (allowedTextNodes.has(rawText)) {
          continue;
        }
        if (allowedTextPatterns.some((pattern) => pattern.test(rawText))) {
          continue;
        }
        findings.push(`${path.relative(sourceDir, filePath)}:${index + 1}: ${rawText}`);
      }

      for (const match of line.matchAll(attributePattern)) {
        findings.push(
          `${path.relative(sourceDir, filePath)}:${index + 1}: ${match[1]}="${match[3]}"`,
        );
      }
    }
  }

  return findings;
}

function collectUnexpectedEnglishCatalogClones(): string[] {
  const allowedKeys = new Set([
    "detail.chat",
    "history.file",
    "inspector.audio",
    "inspector.file",
    "inspector.format",
    "audio.timeUnit.hours",
    "audio.timeUnit.minutes",
    "audio.timeUnit.seconds",
    "settings.whisper.bestOf",
    "settings.whisper.threads",
    "settings.whisperkit.compute.cpu_and_gpu",
    "settings.whisperkit.compute.cpu_and_neural_engine",
    "audio.trimStart",
    "export.format",
    "realtime.status",
    "nav.whisperCpp",
    "settings.localModels.whisperCli",
    "settings.localModels.whisperStream",
    "settings.whisper.title",
    "settings.ai.foundationModel",
  ]);
  const findings: string[] = [];

  for (const language of supportedAppLanguages) {
    if (language === "en") {
      continue;
    }

    for (const [key, englishValue] of Object.entries(translationsCatalog.en)) {
      const translatedValue = translationsCatalog[language][key];
      if (
        englishValue === translatedValue
        && /\s/.test(englishValue)
        && !allowedKeys.has(key)
      ) {
        findings.push(`${language}:${key} => ${translatedValue}`);
      }
    }
  }

  return findings;
}

function collectForbiddenAnglicisms(): string[] {
  const forbiddenPatterns = [
    /\bruntime\b/i,
    /\bdownload\b/i,
  ];
  const findings: string[] = [];

  for (const language of supportedAppLanguages) {
    if (language === "en") {
      continue;
    }

    for (const [key, translatedValue] of Object.entries(translationsCatalog[language])) {
      if (forbiddenPatterns.some((pattern) => pattern.test(translatedValue))) {
        findings.push(`${language}:${key} => ${translatedValue}`);
      }
    }
  }

  return findings;
}

describe("i18n catalog", () => {
  it("keeps supported app languages aligned with the translation catalogs", () => {
    expect(Object.keys(translationsCatalog).sort()).toEqual([...supportedAppLanguages].sort());
  });

  it("keeps the same key set across all supported languages", () => {
    const [referenceLanguage, referenceEntries] = Object.entries(translationsCatalog)[0];
    const referenceKeys = Object.keys(referenceEntries).sort();

    for (const [language, entries] of Object.entries(translationsCatalog)) {
      const keys = Object.keys(entries).sort();
      expect(
        keys,
        `Translation key drift detected between ${referenceLanguage} and ${language}`,
      ).toEqual(referenceKeys);
    }
  });

  it("contains every literal translation key used in source files", () => {
    const literalKeys = collectLiteralTranslationKeys();

    for (const [language, entries] of Object.entries(translationsCatalog)) {
      const missing = literalKeys.filter((key) => !(key in entries));
      expect(
        missing,
        `Missing translation keys for ${language}: ${missing.join(", ")}`,
      ).toEqual([]);
    }
  });

  it("does not leave raw visible JSX copy outside the translation system", () => {
    const findings = collectRawVisibleJsxText();

    expect(
      findings,
      `Raw visible JSX copy detected outside i18n:\n${findings.join("\n")}`,
    ).toEqual([]);
  });

  it("does not reuse unexpected multiword English copy in non-English catalogs", () => {
    const findings = collectUnexpectedEnglishCatalogClones();

    expect(
      findings,
      `Unexpected English copy reused outside the English catalog:\n${findings.join("\n")}`,
    ).toEqual([]);
  });

  it("does not keep forbidden standalone anglicisms in non-English catalogs", () => {
    const findings = collectForbiddenAnglicisms();

    expect(
      findings,
      `Forbidden anglicisms found in non-English catalogs:\n${findings.join("\n")}`,
    ).toEqual([]);
  });
});
