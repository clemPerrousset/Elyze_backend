use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Token format: HMAC-SHA256(phone_id + ":" + YYYYMMDD, secret)
/// We accept today and yesterday to handle timezone edge cases.
///
/// If secret is set to "DISABLED", validation is skipped (dev mode only).
pub fn verify_phone_token(phone_id: &str, token: &str, secret: &str) -> bool {
    if secret == "DISABLED" {
        return true;
    }

    let now = Utc::now();
    let dates = [
        now.format("%Y%m%d").to_string(),
        (now - chrono::Duration::days(1))
            .format("%Y%m%d")
            .to_string(),
    ];

    for date in &dates {
        let message = format!("{}:{}", phone_id, date);
        if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
            mac.update(message.as_bytes());
            let result = hex::encode(mac.finalize().into_bytes());
            if result == token {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_token(phone_id: &str, secret: &str, date: &str) -> String {
        let message = format!("{}:{}", phone_id, date);
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn valid_token_today() {
        let secret = "test_secret";
        let phone_id = "device_abc123";
        let date = Utc::now().format("%Y%m%d").to_string();
        let token = make_token(phone_id, secret, &date);
        assert!(verify_phone_token(phone_id, &token, secret));
    }

    #[test]
    fn valid_token_yesterday() {
        let secret = "test_secret";
        let phone_id = "device_abc123";
        let yesterday = (Utc::now() - chrono::Duration::days(1))
            .format("%Y%m%d")
            .to_string();
        let token = make_token(phone_id, secret, &yesterday);
        assert!(verify_phone_token(phone_id, &token, secret));
    }

    #[test]
    fn invalid_token_wrong_secret() {
        let phone_id = "device_abc123";
        let date = Utc::now().format("%Y%m%d").to_string();
        let token = make_token(phone_id, "wrong_secret", &date);
        assert!(!verify_phone_token(phone_id, &token, "real_secret"));
    }

    #[test]
    fn invalid_token_wrong_phone_id() {
        let secret = "test_secret";
        let date = Utc::now().format("%Y%m%d").to_string();
        let token = make_token("device_real", secret, &date);
        assert!(!verify_phone_token("device_fake", &token, secret));
    }

    #[test]
    fn invalid_token_expired_two_days_ago() {
        let secret = "test_secret";
        let phone_id = "device_abc123";
        let old_date = (Utc::now() - chrono::Duration::days(2))
            .format("%Y%m%d")
            .to_string();
        let token = make_token(phone_id, secret, &old_date);
        assert!(!verify_phone_token(phone_id, &token, secret));
    }

    #[test]
    fn disabled_mode_always_passes() {
        assert!(verify_phone_token("any_id", "any_token", "DISABLED"));
    }
}
