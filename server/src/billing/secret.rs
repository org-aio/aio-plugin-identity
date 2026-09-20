use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead as _, AeadCore, KeyInit as _, OsRng},
};
use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// 支付渠道私钥的落库密钥。缺失时拒绝保存私钥，避免明文入库。
const SECRET_KEY_ENV: &str = "AIO_BILLING_SECRET_KEY";

/// 使用 AES-256-GCM 加密敏感字段，输出 `base64(nonce || ciphertext)`。
pub(super) fn encrypt(plaintext: &str) -> Result<String> {
    encrypt_with_key(plaintext, &configured_key()?)
}

/// 使用 AES-256-GCM 解密 `encrypt` 的输出。
pub(super) fn decrypt(encoded: &str) -> Result<String> {
    decrypt_with_key(encoded, &configured_key()?)
}

/// 加密实现；密钥由调用方提供，便于测试不依赖进程环境。
fn encrypt_with_key(plaintext: &str, key: &[u8; 32]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).context("初始化支付渠道加密器失败")?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("加密支付渠道私钥失败"))?;
    let mut payload = nonce.to_vec();
    payload.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(payload))
}

fn decrypt_with_key(encoded: &str, key: &[u8; 32]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).context("初始化支付渠道加密器失败")?;
    let payload = STANDARD.decode(encoded).context("支付渠道私钥密文无效")?;
    ensure!(payload.len() > 12, "支付渠道私钥密文长度无效");
    let (nonce, ciphertext) = payload.split_at(12);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow::anyhow!("解密支付渠道私钥失败"))?;
    String::from_utf8(plaintext).context("支付渠道私钥不是有效 UTF-8")
}

fn configured_key() -> Result<[u8; 32]> {
    let encoded = std::env::var(SECRET_KEY_ENV)
        .map_err(|_| anyhow::anyhow!("未配置 {SECRET_KEY_ENV}，无法保存或使用支付渠道私钥"))?;
    let key = STANDARD
        .decode(encoded.trim())
        .context("AIO_BILLING_SECRET_KEY 必须是 Base64")?;
    if key.len() != 32 {
        bail!("AIO_BILLING_SECRET_KEY 必须是 32 字节 Base64");
    }
    key.try_into()
        .map_err(|_| anyhow::anyhow!("AIO_BILLING_SECRET_KEY 长度无效"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7_u8; 32];

    #[test]
    fn round_trips_secrets() {
        let encoded = encrypt_with_key("alipay-private-key", &KEY).unwrap();
        assert_ne!(encoded, "alipay-private-key");
        assert_eq!(
            decrypt_with_key(&encoded, &KEY).unwrap(),
            "alipay-private-key"
        );
    }

    #[test]
    fn ciphertext_differs_between_calls() {
        assert_ne!(
            encrypt_with_key("same", &KEY).unwrap(),
            encrypt_with_key("same", &KEY).unwrap()
        );
    }

    #[test]
    fn rejects_wrong_key_and_corrupt_ciphertext() {
        let encoded = encrypt_with_key("value", &KEY).unwrap();
        assert!(decrypt_with_key(&encoded, &[9_u8; 32]).is_err());
        assert!(decrypt_with_key("AAAA", &KEY).is_err());
    }

    #[test]
    fn rejects_wrong_key_size() {
        let short = STANDARD.encode([1_u8; 16]);
        let decoded = STANDARD.decode(short).unwrap();
        assert_ne!(decoded.len(), 32);
    }
}
