use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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
            "{message}{availability}"
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
            "{template}\n\nLanguage: {language_instruction}\n\nTranscript:\n{text}\n\nReturn only the optimized text."
        );
    }

    format!(
        "Clean and optimize this transcript while preserving the same language as the source text ({language_instruction}). Return only optimized text.\n\n{text}"
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
        "Generate in language {language_code}:\n1) Summary\n2) Exactly 3 FAQs with answers.\nFormat:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\nText:\n{text}"
    )
}

fn run_foundation_bridge(
    input: &FoundationBridgeInput,
) -> Result<FoundationBridgeOutput, ApplicationError> {
    let binary_path = ensure_bridge_binary()?;
    let input_json = serde_json::to_vec(input).map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "failed to encode Foundation bridge input: {error}"
        ))
    })?;

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

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(&input_json).map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to write Foundation bridge input: {error}"
            ))
        })?;
    }

    let output = child.wait_with_output().map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "failed to read Foundation bridge output: {error}"
        ))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("Foundation bridge exited with status {}", output.status)
        } else {
            format!("Foundation bridge failed: {stderr}")
        };
        return Err(ApplicationError::PostProcessing(message));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "Foundation bridge returned invalid UTF-8 output: {error}"
        ))
    })?;

    serde_json::from_str::<FoundationBridgeOutput>(stdout.trim()).map_err(|error| {
        ApplicationError::PostProcessing(format!(
            "failed to decode Foundation bridge response: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::build_optimize_prompt;

    #[test]
    fn optimize_prompt_defaults_to_source_language_when_auto() {
        let prompt = build_optimize_prompt("ciao", "auto", None, None);
        assert!(prompt.contains("the same language as the source text"));
        assert!(prompt.contains("the same language as the transcript"));
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

    let compile_output = Command::new("xcrun")
        .arg("swiftc")
        .arg("-parse-as-library")
        .arg(&script_path)
        .arg("-o")
        .arg(&binary_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            ApplicationError::PostProcessing(format!(
                "failed to compile Foundation bridge with swiftc: {error}"
            ))
        })?;

    if !compile_output.status.success() {
        let stderr = String::from_utf8_lossy(&compile_output.stderr)
            .trim()
            .to_string();
        let stdout = String::from_utf8_lossy(&compile_output.stdout)
            .trim()
            .to_string();
        let diagnostics = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("swiftc exited with status {}", compile_output.status)
        };
        return Err(ApplicationError::PostProcessing(format!(
            "Foundation bridge failed to compile: {diagnostics}"
        )));
    }

    let _ = BINARY_PATH.set(binary_path.clone());
    Ok(binary_path)
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
        do {
            let data = FileHandle.standardInput.readDataToEndOfFile()
            let input = try JSONDecoder().decode(BridgeInput.self, from: data)

            #if canImport(FoundationModels)
            let model = SystemLanguageModel.default
            guard model.isAvailable else {
                let output = BridgeOutput(
                    ok: false,
                    content: nil,
                    error: "Foundation Model is unavailable on this Mac",
                    availability: availabilityDescription(model.availability)
                )
                print(encode(output))
                return
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
            let output = BridgeOutput(
                ok: true,
                content: response.content,
                error: nil,
                availability: "available"
            )
            print(encode(output))
            #else
            let output = BridgeOutput(
                ok: false,
                content: nil,
                error: "FoundationModels framework is not available in this runtime",
                availability: "unsupported_runtime"
            )
            print(encode(output))
            #endif
        } catch {
            let output = BridgeOutput(
                ok: false,
                content: nil,
                error: "Foundation bridge error: \(error.localizedDescription)",
                availability: nil
            )
            print(encode(output))
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
}
"#;
