use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use sbobino_application::{dto::SummaryFaq, ApplicationError, TranscriptEnhancer};

#[derive(Debug, Clone)]
pub struct FoundationAppleEnhancer {
    optimize_prompt_override: Option<String>,
    summary_prompt_override: Option<String>,
}

impl FoundationAppleEnhancer {
    pub fn new(
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Self {
        Self {
            optimize_prompt_override: normalize_prompt(optimize_prompt_override),
            summary_prompt_override: normalize_prompt(summary_prompt_override),
        }
    }

    fn generate(&self, prompt: &str) -> Result<String, ApplicationError> {
        if !cfg!(target_os = "macos") {
            return Err(ApplicationError::PostProcessing(
                "Apple Foundation Model provider is only available on macOS".to_string(),
            ));
        }

        let input = FoundationBridgeInput {
            prompt: prompt.to_string(),
            instructions: None,
        };
        let output = run_foundation_bridge(&input)?;
        if output.ok {
            let content = output.content.unwrap_or_default();
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return Err(ApplicationError::PostProcessing(
                    "foundation model response was empty".to_string(),
                ));
            }
            return Ok(trimmed.to_string());
        }

        let availability = output
            .availability
            .as_deref()
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        let message = output
            .error
            .unwrap_or_else(|| "Foundation model request failed".to_string());
        Err(ApplicationError::PostProcessing(format!(
            "{}{availability}",
            normalize_foundation_runtime_error(&message)
        )))
    }

    pub async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        self.generate(prompt)
    }

    pub async fn optimize_with_prompt(
        &self,
        text: &str,
        language_code: &str,
        prompt_override: Option<&str>,
    ) -> Result<String, ApplicationError> {
        let prompt = build_optimize_prompt(
            text,
            language_code,
            prompt_override,
            self.optimize_prompt_override.as_deref(),
        );
        self.generate(&prompt)
    }

    pub async fn summarize_and_faq_with_prompt(
        &self,
        text: &str,
        language_code: &str,
        prompt_override: Option<&str>,
    ) -> Result<SummaryFaq, ApplicationError> {
        let prompt = build_summary_prompt(
            text,
            language_code,
            prompt_override,
            self.summary_prompt_override.as_deref(),
        );
        let output = self.generate(&prompt)?;

        let (summary, faqs) = if let Some((left, right)) = output.split_once("FAQs:") {
            (
                left.replace("Summary:", "").trim().to_string(),
                right.trim().to_string(),
            )
        } else {
            (output.trim().to_string(), String::new())
        };

        Ok(SummaryFaq { summary, faqs })
    }
}

#[async_trait]
impl TranscriptEnhancer for FoundationAppleEnhancer {
    async fn optimize(&self, text: &str, language_code: &str) -> Result<String, ApplicationError> {
        self.optimize_with_prompt(text, language_code, None).await
    }

    async fn summarize_and_faq(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError> {
        self.summarize_and_faq_with_prompt(text, language_code, None)
            .await
    }

    async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        FoundationAppleEnhancer::ask(self, prompt).await
    }

    fn prefers_single_pass_summary(&self) -> bool {
        true
    }

    fn summary_chunk_concurrency_limit(&self) -> usize {
        1
    }
}

#[derive(Debug, Serialize)]
struct FoundationBridgeInput {
    prompt: String,
    instructions: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FoundationBridgeOutput {
    ok: bool,
    content: Option<String>,
    error: Option<String>,
    availability: Option<String>,
}

fn normalize_prompt(value: Option<String>) -> Option<String> {
    value.and_then(|prompt| {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn build_optimize_prompt(
    text: &str,
    language_code: &str,
    prompt_override: Option<&str>,
    default_override: Option<&str>,
) -> String {
    let language_instruction = optimize_language_instruction(language_code);
    if let Some(template) = prompt_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            default_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    {
        return format!(
            "{template}\n\nLanguage: {language_instruction}\n\nAdditional cleanup rules:\n- Preserve the original wording, structure, and order of the transcript as much as possible.\n- Improve punctuation, capitalization, spacing, and paragraph breaks.\n- Remove obvious accidental repetitions, duplicated lines, and looped sentences.\n- Keep only one occurrence when the same sentence is repeated in sequence by mistake.\n- Correct isolated words or short phrases that are clearly wrong ASR/transcription mistakes when the surrounding context makes the intended meaning highly likely.\n- Prefer minimal local corrections, especially for technical terms, acronyms, library names, product names, and domain-specific jargon.\n- If you are not confident about a correction, keep the original wording.\n- Do not paraphrase whole sentences, summarize, reorder ideas, or invent missing facts.\n\nTranscript:\n{text}\n\nReturn only the cleaned transcript."
        );
    }

    format!(
        "Clean this transcript while preserving the same language as the source text ({language_instruction}). Preserve the original wording, structure, and order as much as possible. Improve punctuation, capitalization, spacing, and paragraph breaks, and remove obvious transcription glitches such as consecutive duplicated lines, repeated phrases, looped sentences, and hallucinated filler. When the same sentence is repeated accidentally in sequence, keep only the single best occurrence. You may correct isolated words or short phrases that are clearly wrong ASR/transcription mistakes when the surrounding context makes the intended term highly likely, especially for technical terms, acronyms, library names, product names, and domain-specific jargon. Prefer minimal local corrections. If uncertain, keep the original wording. Do not paraphrase whole sentences, summarize, reorder ideas, or invent missing facts. Return only the cleaned transcript.\n\n{text}"
    )
}

fn optimize_language_instruction(language_code: &str) -> &str {
    let normalized = language_code.trim();
    if normalized.is_empty() || normalized == "auto" {
        "the same language as the transcript"
    } else {
        normalized
    }
}

fn build_summary_prompt(
    text: &str,
    language_code: &str,
    prompt_override: Option<&str>,
    default_override: Option<&str>,
) -> String {
    if let Some(template) = prompt_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            default_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    {
        return format!(
            "{template}\n\nLanguage: {language_code}\n\nTranscript:\n{text}\n\nFormat strictly as:\nSummary:\n...\nFAQs:\nQ:...\nA:..."
        );
    }

    format!(
        "Generate in language {language_code}:\n1) Summary\n2) Exactly 3 FAQs with answers.\n\nSummary requirements:\n- Write a detailed, sectioned briefing note, not a terse recap.\n- Cover all major topics, technical details, examples, numbers, and decisions.\n- Preserve how the ideas relate to each other and explain why they matter.\n- Keep the summary self-contained for a reader who has not heard the recording.\n\nFormat:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\nText:\n{text}"
    )
}

fn run_foundation_bridge(
    input: &FoundationBridgeInput,
) -> Result<FoundationBridgeOutput, ApplicationError> {
    static CLIENT: OnceLock<Mutex<Option<FoundationBridgeProcess>>> = OnceLock::new();
    let client = CLIENT.get_or_init(|| Mutex::new(None));
    let mut guard = client.lock().map_err(|_| {
        ApplicationError::PostProcessing("Foundation bridge client lock poisoned".to_string())
    })?;

    if guard.is_none() {
        *guard = Some(FoundationBridgeProcess::spawn()?);
    }

    let first_attempt = guard
        .as_mut()
        .expect("foundation bridge process initialized")
        .send(input);

    match first_attempt {
        Ok(output) => Ok(output),
        Err(_) => {
            *guard = Some(FoundationBridgeProcess::spawn()?);
            guard
                .as_mut()
                .expect("foundation bridge process reinitialized")
                .send(input)
        }
    }
}

struct FoundationBridgeProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl FoundationBridgeProcess {
    fn spawn() -> Result<Self, ApplicationError> {
        let binary_path = ensure_bridge_binary()?;
        let mut child = Command::new(binary_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                ApplicationError::PostProcessing(format!(
                    "failed to launch Foundation bridge binary: {error}"
                ))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ApplicationError::PostProcessing(
                "Foundation bridge did not expose a writable stdin".to_string(),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::PostProcessing(
                "Foundation bridge did not expose a readable stdout".to_string(),
            )
        })?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn send(
        &mut self,
        input: &FoundationBridgeInput,
    ) -> Result<FoundationBridgeOutput, ApplicationError> {
        let input_json = serde_json::to_string(input).map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to encode Foundation bridge input: {error}"
            ))
        })?;

        writeln!(self.stdin, "{input_json}").map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to write Foundation bridge input: {error}"
            ))
        })?;
        self.stdin.flush().map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to flush Foundation bridge input: {error}"
            ))
        })?;

        let mut response_line = String::new();
        let bytes_read = self.stdout.read_line(&mut response_line).map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to read Foundation bridge output: {error}"
            ))
        })?;

        if bytes_read == 0 {
            let status = self.child.try_wait().ok().flatten();
            let suffix = status
                .map(|value| format!(" (status {value})"))
                .unwrap_or_default();
            return Err(ApplicationError::PostProcessing(format!(
                "Foundation bridge terminated without a response{suffix}"
            )));
        }

        serde_json::from_str::<FoundationBridgeOutput>(response_line.trim()).map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to decode Foundation bridge response: {error}"
            ))
        })
    }
}

impl Drop for FoundationBridgeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::{build_optimize_prompt, normalize_foundation_runtime_error};

    #[test]
    fn optimize_prompt_defaults_to_source_language_when_auto() {
        let prompt = build_optimize_prompt("ciao", "auto", None, None);
        assert!(prompt.contains("the same language as the source text"));
        assert!(prompt.contains("the same language as the transcript"));
        assert!(prompt.contains("repeated phrases"));
    }

    #[test]
    fn foundation_generation_error_is_made_actionable() {
        let message = normalize_foundation_runtime_error(
            "Foundation bridge error: The operation couldn’t be completed. (FoundationModels.LanguageModelSession.GenerationError error -1.)",
        );
        assert!(message.contains("Switch AI Service"));
        assert!(message.contains("compatible Xcode toolchain"));
    }
}

fn ensure_bridge_script() -> Result<PathBuf, ApplicationError> {
    static SCRIPT_PATH: OnceLock<PathBuf> = OnceLock::new();

    if let Some(path) = SCRIPT_PATH.get() {
        return Ok(path.clone());
    }

    let dir = std::env::temp_dir().join("sbobino_foundation");
    std::fs::create_dir_all(&dir).map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "failed to create Foundation bridge temp directory: {error}"
        ))
    })?;

    let path = dir.join("foundation_bridge.swift");
    let should_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != FOUNDATION_BRIDGE_SWIFT,
        Err(_) => true,
    };

    if should_write {
        std::fs::write(&path, FOUNDATION_BRIDGE_SWIFT).map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to write Foundation bridge script: {error}"
            ))
        })?;
    }

    let _ = SCRIPT_PATH.set(path.clone());
    Ok(path)
}

fn ensure_bridge_binary() -> Result<PathBuf, ApplicationError> {
    static BINARY_PATH: OnceLock<PathBuf> = OnceLock::new();

    if let Some(path) = BINARY_PATH.get() {
        return Ok(path.clone());
    }

    let script_path = ensure_bridge_script()?;
    let binary_path = script_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("foundation_bridge_bin");
    let module_cache_path = script_path
        .parent()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join("module-cache");

    std::fs::create_dir_all(&module_cache_path).map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "failed to create Foundation bridge module cache: {error}"
        ))
    })?;

    let mut diagnostics = Vec::new();
    for candidate in foundation_swiftc_candidates() {
        let mut command = Command::new(&candidate.program);
        if let Some(developer_dir) = candidate.developer_dir.as_ref() {
            command.env("DEVELOPER_DIR", developer_dir);
        }

        let compile_output = command
            .args(&candidate.pre_args)
            .arg("-module-cache-path")
            .arg(&module_cache_path)
            .arg("-parse-as-library")
            .arg(&script_path)
            .arg("-o")
            .arg(&binary_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| {
                ApplicationError::PostProcessing(format!(
                    "failed to launch Foundation bridge compiler ({}): {error}",
                    candidate.label
                ))
            })?;

        if compile_output.status.success() {
            let _ = BINARY_PATH.set(binary_path.clone());
            return Ok(binary_path);
        }

        let stderr = String::from_utf8_lossy(&compile_output.stderr)
            .trim()
            .to_string();
        let stdout = String::from_utf8_lossy(&compile_output.stdout)
            .trim()
            .to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("swiftc exited with status {}", compile_output.status)
        };
        diagnostics.push(format!(
            "{}: {}",
            candidate.label,
            summarize_compile_diagnostics(&detail)
        ));
    }

    Err(ApplicationError::PostProcessing(format!(
        "Foundation bridge failed to compile. Select a compatible Xcode toolchain or switch AI Service. {}",
        diagnostics.join(" | ")
    )))
}

#[derive(Debug, Clone)]
struct SwiftcCandidate {
    label: String,
    program: PathBuf,
    pre_args: Vec<String>,
    developer_dir: Option<PathBuf>,
}

fn foundation_swiftc_candidates() -> Vec<SwiftcCandidate> {
    let mut candidates = Vec::new();

    let xcode_developer_dir = PathBuf::from("/Applications/Xcode.app/Contents/Developer");
    if xcode_developer_dir.is_dir() {
        candidates.push(SwiftcCandidate {
            label: "Xcode xcrun swiftc".to_string(),
            program: PathBuf::from("xcrun"),
            pre_args: vec![
                "--sdk".to_string(),
                "macosx".to_string(),
                "swiftc".to_string(),
            ],
            developer_dir: Some(xcode_developer_dir),
        });
    }

    candidates.push(SwiftcCandidate {
        label: "xcrun swiftc".to_string(),
        program: PathBuf::from("xcrun"),
        pre_args: vec![
            "--sdk".to_string(),
            "macosx".to_string(),
            "swiftc".to_string(),
        ],
        developer_dir: None,
    });

    candidates
}

fn summarize_compile_diagnostics(detail: &str) -> String {
    let normalized = detail.replace('\n', " ");
    if normalized.contains("this SDK is not supported by the compiler") {
        return "toolchain/SDK mismatch while compiling FoundationModels".to_string();
    }
    if normalized.contains("ModuleCache") && normalized.contains("Operation not permitted") {
        return "module cache was not writable during FoundationModels compilation".to_string();
    }
    normalized
        .split_whitespace()
        .take(32)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_foundation_runtime_error(message: &str) -> String {
    let normalized = message.trim();
    if normalized.contains("LanguageModelSession.GenerationError error -1") {
        return "Foundation Model request failed on this Mac. Switch AI Service from Foundation Model to a configured remote provider, or select a compatible Xcode toolchain and relaunch the app.".to_string();
    }
    normalized.to_string()
}

const FOUNDATION_BRIDGE_SWIFT: &str = r#"
import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

struct BridgeInput: Decodable {
    let prompt: String
    let instructions: String?
}

struct BridgeOutput: Encodable {
    let ok: Bool
    let content: String?
    let error: String?
    let availability: String?
}

@main
struct FoundationBridge {
    static func main() async {
        while let line = readLine() {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty {
                continue
            }

            do {
                guard let data = trimmed.data(using: .utf8) else {
                    writeLine(
                        BridgeOutput(
                            ok: false,
                            content: nil,
                            error: "Foundation bridge error: invalid UTF-8 input",
                            availability: nil
                        )
                    )
                    continue
                }

                let input = try JSONDecoder().decode(BridgeInput.self, from: data)

                #if canImport(FoundationModels)
                let model = SystemLanguageModel.default
                guard model.isAvailable else {
                    writeLine(
                        BridgeOutput(
                            ok: false,
                            content: nil,
                            error: "Foundation Model is unavailable on this Mac",
                            availability: availabilityDescription(model.availability)
                        )
                    )
                    continue
                }

                let session = LanguageModelSession()
                let mergedPrompt: String
                if let instructions = input.instructions?.trimmingCharacters(in: .whitespacesAndNewlines),
                   !instructions.isEmpty {
                    mergedPrompt = "\(instructions)\n\n\(input.prompt)"
                } else {
                    mergedPrompt = input.prompt
                }

                let response = try await session.respond(to: mergedPrompt)
                writeLine(
                    BridgeOutput(
                        ok: true,
                        content: response.content,
                        error: nil,
                        availability: "available"
                    )
                )
                #else
                writeLine(
                    BridgeOutput(
                        ok: false,
                        content: nil,
                        error: "FoundationModels framework is not available in this runtime",
                        availability: "unsupported_runtime"
                    )
                )
                #endif
            } catch {
                writeLine(
                    BridgeOutput(
                        ok: false,
                        content: nil,
                        error: "Foundation bridge error: \(error.localizedDescription)",
                        availability: nil
                    )
                )
            }
        }
    }

    #if canImport(FoundationModels)
    static func availabilityDescription(_ availability: SystemLanguageModel.Availability) -> String {
        switch availability {
        case .available:
            return "available"
        case .unavailable(let reason):
            switch reason {
            case .deviceNotEligible:
                return "device_not_eligible"
            case .appleIntelligenceNotEnabled:
                return "apple_intelligence_not_enabled"
            case .modelNotReady:
                return "model_not_ready"
            @unknown default:
                return "unavailable"
            }
        }
    }
    #endif

    static func encode(_ value: BridgeOutput) -> String {
        let encoder = JSONEncoder()
        if let data = try? encoder.encode(value), let text = String(data: data, encoding: .utf8) {
            return text
        }
        return "{\"ok\":false,\"error\":\"encoding_failure\"}"
    }

    static func writeLine(_ value: BridgeOutput) {
        let line = encode(value) + "\n"
        if let data = line.data(using: .utf8) {
            try? FileHandle.standardOutput.write(contentsOf: data)
        }
    }
}
"#;
