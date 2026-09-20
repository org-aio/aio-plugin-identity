use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rsa::{
    Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey,
    pkcs1::{DecodeRsaPrivateKey as _, DecodeRsaPublicKey as _},
    pkcs8::{DecodePrivateKey as _, DecodePublicKey as _},
};
use sha2::{Digest as _, Sha256};

/// 支付宝网关默认地址（正式环境）。
pub(super) const DEFAULT_GATEWAY: &str = "https://openapi.alipay.com/gateway.do";

/// 按支付宝规则拼接待签名串：过滤空值与 `sign`，按字典序升序，用 `&` 连接。
pub(super) fn signing_content<'a>(fields: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    build_content(fields, false)
}

/// 异步通知验签的待签名串：支付宝要求同时排除 `sign` 与 `sign_type`。
pub(super) fn notification_content<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    build_content(fields, true)
}

fn build_content<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a str)>,
    exclude_sign_type: bool,
) -> String {
    let mut pairs = fields
        .into_iter()
        .filter(|(key, value)| {
            *key != "sign" && !(exclude_sign_type && *key == "sign_type") && !value.is_empty()
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// 使用商户私钥做 RSA2(SHA256withRSA) 签名，返回 Base64。
pub(super) fn sign(content: &str, private_key: &str) -> Result<String> {
    let key = parse_private_key(private_key)?;
    let digest = Sha256::digest(content.as_bytes());
    let signature = key
        .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
        .context("支付宝请求签名失败")?;
    Ok(STANDARD.encode(signature))
}

/// 使用支付宝公钥校验回调签名。
pub(super) fn verify(content: &str, signature: &str, public_key: &str) -> Result<()> {
    let key = parse_public_key(public_key)?;
    let signature = STANDARD
        .decode(signature)
        .context("支付宝回调签名不是有效 Base64")?;
    let digest = Sha256::digest(content.as_bytes());
    key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
        .context("支付宝回调验签失败")?;
    Ok(())
}

/// 电脑网站支付所需的渠道与订单参数。
pub(super) struct PagePay<'a> {
    pub gateway: &'a str,
    pub app_id: &'a str,
    pub notify_url: &'a str,
    pub return_url: &'a str,
    pub out_trade_no: &'a str,
    pub amount_yuan: &'a str,
    pub subject: &'a str,
    pub private_key: &'a str,
}

/// 生成电脑网站支付的跳转地址，参数已带签名。
pub(super) fn page_pay_url(pay: PagePay<'_>) -> Result<String> {
    let PagePay {
        gateway,
        app_id,
        notify_url,
        return_url,
        out_trade_no,
        amount_yuan,
        subject,
        private_key,
    } = pay;
    ensure!(!app_id.is_empty(), "支付宝 app_id 未配置");
    ensure!(!private_key.is_empty(), "支付宝应用私钥未配置");
    // biz_content 与 timestamp 需要单独构造后再参与签名。
    let biz_content = format!(
        r#"{{"out_trade_no":"{out_trade_no}","product_code":"FAST_INSTANT_TRADE_PAY","total_amount":"{amount_yuan}","subject":"{subject}"}}"#
    );
    let timestamp = chrono_like_timestamp();
    let signed = signing_content([
        ("app_id", app_id),
        ("biz_content", &biz_content),
        ("charset", "utf-8"),
        ("format", "JSON"),
        ("method", "alipay.trade.page.pay"),
        ("notify_url", notify_url),
        ("return_url", return_url),
        ("sign_type", "RSA2"),
        ("timestamp", &timestamp),
        ("version", "1.0"),
    ]);
    let signature = sign(&signed, private_key)?;
    let encoded = |value: &str| urlencode(value);
    Ok(format!(
        "{gateway}?app_id={}&biz_content={}&charset=utf-8&format=JSON&method=alipay.trade.page.pay&notify_url={}&return_url={}&sign_type=RSA2&timestamp={}&version=1.0&sign={}",
        encoded(app_id),
        encoded(&biz_content),
        encoded(notify_url),
        encoded(return_url),
        encoded(&timestamp),
        encoded(&signature),
    ))
}

fn parse_private_key(value: &str) -> Result<RsaPrivateKey> {
    let normalized = normalize_pem(value);
    if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(&normalized) {
        return Ok(key);
    }
    RsaPrivateKey::from_pkcs1_pem(&normalized).context("支付宝应用私钥格式无效")
}

fn parse_public_key(value: &str) -> Result<RsaPublicKey> {
    let normalized = normalize_pem(value);
    if let Ok(key) = RsaPublicKey::from_public_key_pem(&normalized) {
        return Ok(key);
    }
    RsaPublicKey::from_pkcs1_pem(&normalized).context("支付宝公钥格式无效")
}

/// 接受裸 Base64 或 PEM；裸值按 PKCS#8 私钥 / 公钥补全 PEM 头。
fn normalize_pem(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.contains("-----BEGIN") {
        return trimmed.to_owned();
    }
    if trimmed.len() > 300 {
        format!("-----BEGIN PRIVATE KEY-----\n{trimmed}\n-----END PRIVATE KEY-----")
    } else {
        format!("-----BEGIN PUBLIC KEY-----\n{trimmed}\n-----END PUBLIC KEY-----")
    }
}

/// 支付宝要求 `yyyy-MM-dd HH:mm:ss`，使用 UTC+8 的本地时间。
fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
        + 8 * 3600;
    let (year, month, day, hour, minute, second) = civil_from_seconds(seconds as i64);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// 由 Unix 秒数换算公历日期，避免额外引入时间库。
fn civil_from_seconds(seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant 的 civil_from_days 算法。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d, hour, minute, second)
}

/// 最小百分号编码，用于查询串参数。
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_owned(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// 解析支付宝异步通知的表单参数并验签。
pub(super) fn verify_notification(fields: &[(String, String)], public_key: &str) -> Result<String> {
    let signature = fields
        .iter()
        .find(|(key, _)| key == "sign")
        .map(|(_, value)| value.clone())
        .context("支付宝回调缺少 sign")?;
    let content = notification_content(
        fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    verify(&content, &signature, public_key)?;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_content_sorts_and_drops_empty_values() {
        let content = signing_content([
            ("b", "2"),
            ("a", "1"),
            ("empty", ""),
            ("sign", "ignored"),
            ("sign_type", "RSA2"),
            ("c", "3"),
        ]);
        assert_eq!(content, "a=1&b=2&c=3&sign_type=RSA2");
    }

    #[test]
    fn notification_content_excludes_sign_and_sign_type() {
        let content = notification_content([
            ("trade_status", "TRADE_SUCCESS"),
            ("sign_type", "RSA2"),
            ("sign", "ignored"),
        ]);
        assert_eq!(content, "trade_status=TRADE_SUCCESS");
    }

    #[test]
    fn civil_conversion_matches_known_dates() {
        // 2026-09-20 16:00:00 UTC 对应 1789920000 Unix 秒；加 8 小时偏移后
        // 结果应表示 UTC+8 的 2026-09-21 00:00:00。
        let (y, m, d, h, min, s) = civil_from_seconds(1_789_920_000 + 8 * 3600);
        assert_eq!((y, m, d, h, min, s), (2026, 9, 21, 0, 0, 0));
        // 再验证一个整点：1970-01-01 00:00:00 UTC。
        assert_eq!(civil_from_seconds(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn urlencode_escapes_reserved_characters() {
        assert_eq!(urlencode("a b&c=d"), "a+b%26c%3Dd");
        assert_eq!(urlencode("AZaz09-_.~"), "AZaz09-_.~");
    }
}
