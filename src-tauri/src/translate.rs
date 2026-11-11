//! コピーしたテキストの翻訳
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;

/// OS の証明書ストアで検証する通信クライアント。
/// ウイルス対策ソフトや社内プロキシが HTTPS を中継している環境でも、
/// それらが OS に入れた証明書を信頼できる（ureq 既定の内蔵ルートだと
/// "invalid peer certificate: UnknownIssuer" で失敗する）
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .new_agent()
});

/// 通信エラーを人が読める説明にする
fn explain(e: ureq::Error) -> String {
    let raw = e.to_string();
    let lower = raw.to_ascii_lowercase();
    let hint = if lower.contains("certificate") {
        "（HTTPS の証明書を確認できませんでした。ウイルス対策ソフトや社内プロキシの HTTPS 検査が原因の可能性があります）"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "（応答がありませんでした）"
    } else if lower.contains("dns") || lower.contains("resolve") || lower.contains("connect") {
        "（インターネットに接続できませんでした）"
    } else if lower.contains("429") {
        "（短時間に使いすぎたため一時的に制限されています）"
    } else {
        ""
    };
    format!("{raw}{hint}")
}

/// ひらがな・カタカナ・漢字が多ければ日本語とみなす
pub fn looks_japanese(s: &str) -> bool {
    let (mut ja, mut total) = (0usize, 0usize);
    for c in s.chars().filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation()) {
        total += 1;
        if matches!(c as u32, 0x3040..=0x30FF | 0x4E00..=0x9FFF | 0xFF66..=0xFF9F) {
            ja += 1;
        }
    }
    total > 0 && ja * 10 >= total * 3
}

/// 訳先の言語。日本語の文なら英語へ、それ以外は設定の言語へ
pub fn pick_target(text: &str, preferred: &str) -> String {
    if preferred == "ja" && looks_japanese(text) { "en".into() } else { preferred.into() }
}

pub fn translate(text: &str, provider: &str, deepl_key: &str, target: &str) -> Result<String, String> {
    match provider {
        "deepl" => deepl(text, deepl_key, target),
        _ => google(text, target),
    }
}

/// Google 翻訳（キー不要・非公式エンドポイント。将来使えなくなる可能性あり）
fn google(text: &str, target: &str) -> Result<String, String> {
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={target}&dt=t"
    );
    let v: Value = AGENT
        .post(&url)
        .send_form([("q", text)])
        .map_err(|e| format!("翻訳に失敗しました: {}", explain(e)))?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for seg in v.get(0).and_then(Value::as_array).into_iter().flatten() {
        if let Some(s) = seg.get(0).and_then(Value::as_str) {
            out.push_str(s);
        }
    }
    if out.is_empty() { Err("翻訳結果が空でした".into()) } else { Ok(out) }
}

fn deepl(text: &str, key: &str, target: &str) -> Result<String, String> {
    if key.trim().is_empty() {
        return Err("設定で DeepL の API キーを入力してください".into());
    }
    let host = if key.trim().ends_with(":fx") { "api-free.deepl.com" } else { "api.deepl.com" };
    let lang = match target {
        "en" => "EN-US".to_string(),
        "pt" => "PT-BR".to_string(),
        "zh" => "ZH-HANS".to_string(),
        t => t.to_uppercase(),
    };
    let v: Value = AGENT
        .post(&format!("https://{host}/v2/translate"))
        .header("Authorization", &format!("DeepL-Auth-Key {}", key.trim()))
        .send_json(serde_json::json!({ "text": [text], "target_lang": lang }))
        .map_err(|e| format!("DeepL の翻訳に失敗しました: {}", explain(e)))?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    v["translations"][0]["text"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "DeepL の応答を読めませんでした".into())
}
