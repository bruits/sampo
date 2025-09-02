use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use hmac::{Hmac, Mac};
use octocrab::models::issues::Comment;
use octocrab::models::repos::DiffEntryStatus;
use sha2::Sha256;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Clone)]
struct AppState {
    octo: octocrab::Octocrab,
    webhook_secret: Arc<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sampo_github_bot=debug".into()),
        )
        .init();

    let secret = std::env::var("WEBHOOK_SECRET")
        .or_else(|_| std::env::var("GITHUB_WEBHOOK_SECRET"))
        .expect("WEBHOOK_SECRET env var must be set");

    let octo = match std::env::var("GITHUB_TOKEN") {
        Ok(token) => octocrab::Octocrab::builder()
            .personal_token(token)
            .build()?,
        Err(_) => octocrab::Octocrab::builder().build()?,
    };

    let app_state = AppState {
        octo,
        webhook_secret: Arc::new(secret),
    };

    let app = Router::new()
        .route("/webhook", post(webhook))
        .with_state(app_state);

    let addr: SocketAddr = std::env::var("ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            let port = std::env::var("PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(3000);
            SocketAddr::from(([0, 0, 0, 0], port))
        });

    info!("listening on http://{}", addr);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn webhook(
    State(state): State<AppState>,
    req: Request,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Take headers before consuming body
    let headers = req.headers().clone();
    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Get raw body
    let (_parts, body_inner) = req.into_parts();
    let body = match axum::body::to_bytes(body_inner, 2 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return Err((StatusCode::BAD_REQUEST, format!("invalid body: {e}"))),
    };

    // Verify signature if header present; GitHub apps sign all webhooks
    if let Err(e) = verify_signature(&state.webhook_secret, &headers, &body) {
        warn!("signature verification failed: {}", e);
        return Err((StatusCode::UNAUTHORIZED, "invalid signature".into()));
    }

    if event != "pull_request" {
        return Ok((StatusCode::OK, "ignored"));
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Err((StatusCode::BAD_REQUEST, format!("invalid JSON: {e}"))),
    };

    let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
    // Only respond on relevant PR actions
    let interesting = matches!(
        action,
        "opened" | "synchronize" | "reopened" | "ready_for_review" | "edited"
    );
    if !interesting {
        return Ok((StatusCode::OK, "ignored action"));
    }

    let pr_number = payload
        .get("number")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing PR number".to_string()))?
        as u64;

    // repo info
    let (owner, repo) = match (
        payload
            .get("repository")
            .and_then(|r| r.get("owner"))
            .and_then(|o| o.get("login"))
            .and_then(|v| v.as_str()),
        payload
            .get("repository")
            .and_then(|r| r.get("name"))
            .and_then(|v| v.as_str()),
    ) {
        (Some(o), Some(r)) => (o.to_string(), r.to_string()),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "missing repository owner/name".into(),
            ));
        }
    };

    info!("PR #{} -> {}/{} action={}", pr_number, owner, repo, action);

    // Check files in PR for changeset
    let has_changeset = match pr_has_changeset(&state.octo, &owner, &repo, pr_number).await {
        Ok(v) => v,
        Err(e) => {
            error!("error checking PR files: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to check PR files".into(),
            ));
        }
    };

    // Compose message with a sticky marker to allow updates
    const MARKER: &str = "<!-- sampo-bot:changeset-check -->";
    let body = if has_changeset {
        format!(
            "{marker}\n## 🧭 Changeset detected\n\nMerging this PR will bump the version and include these changes in the next release.\n",
            marker = MARKER
        )
    } else {
        format!(
            "{marker}\n## ⚠️ No changeset detected\n\nIf this PR isn’t meant to release a new version, no action needed. If it should, add a changeset to bump the version.\n",
            marker = MARKER
        )
    };

    if let Err(e) =
        upsert_sticky_comment(&state.octo, &owner, &repo, pr_number, MARKER, &body).await
    {
        error!("failed to upsert comment: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to comment".into(),
        ));
    }

    Ok((StatusCode::OK, "ok"))
}

fn verify_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> Result<(), VerifyError> {
    let sig = headers
        .get("X-Hub-Signature-256")
        .ok_or(VerifyError::MissingHeader)?
        .to_str()
        .map_err(|_| VerifyError::InvalidHeader)?;
    let hex = sig
        .strip_prefix("sha256=")
        .ok_or(VerifyError::InvalidHeader)?;
    let given = decode_hex(hex).ok_or(VerifyError::InvalidHeader)?;

    // Compute HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| VerifyError::Internal("bad secret".into()))?;
    mac.update(body);
    mac.verify_slice(&given).map_err(|_| VerifyError::Mismatch)
}

#[derive(thiserror::Error, Debug)]
enum VerifyError {
    #[error("missing signature header")]
    MissingHeader,
    #[error("invalid signature header")]
    InvalidHeader,
    #[error("signature mismatch")]
    Mismatch,
    #[error("internal: {0}")]
    Internal(String),
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(10 + (c - b'a')),
            b'A'..=b'F' => Some(10 + (c - b'A')),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = val(bytes[i])?;
        let lo = val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

async fn pr_has_changeset(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
) -> octocrab::Result<bool> {
    let files = octo.pulls(owner, repo).list_files(pr).await?;
    let mut page = files;
    let mut any = false;
    let dir_prefix = ".sampo/changesets/"; // align with Sampo defaults
    loop {
        for f in &page {
            let filename = f.filename.as_str();
            let not_removed = !matches!(f.status, DiffEntryStatus::Removed);
            if filename.starts_with(dir_prefix) && filename.ends_with(".md") && not_removed {
                any = true;
                break;
            }
        }
        if any {
            break;
        }
        if let Some(next) = octo
            .get_page::<octocrab::models::repos::DiffEntry>(&page.next)
            .await?
        {
            page = next;
        } else {
            break;
        }
    }
    Ok(any)
}

async fn upsert_sticky_comment(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    marker: &str,
    body: &str,
) -> octocrab::Result<()> {
    // Look for existing comment with marker
    let mut existing: Option<octocrab::models::CommentId> = None;
    let mut page = octo.issues(owner, repo).list_comments(pr).send().await?;
    loop {
        for c in &page {
            if comment_has_marker(c, marker) {
                existing = Some(c.id.clone());
                break;
            }
        }
        if existing.is_some() {
            break;
        }
        if let Some(next) = octo.get_page::<Comment>(&page.next).await? {
            page = next;
        } else {
            break;
        }
    }

    if let Some(id) = existing {
        octo.issues(owner, repo).update_comment(id, body).await?;
    } else {
        octo.issues(owner, repo).create_comment(pr, body).await?;
    }
    Ok(())
}

fn comment_has_marker(c: &Comment, marker: &str) -> bool {
    c.body.as_deref().unwrap_or("").contains(marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_decode_works() {
        assert_eq!(decode_hex("00ff10").unwrap(), vec![0x00, 0xff, 0x10]);
        assert!(decode_hex("0").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn hex_decode_empty_string() {
        assert_eq!(decode_hex("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn hex_decode_lowercase_uppercase() {
        assert_eq!(
            decode_hex("aAbBcCdDeEfF").unwrap(),
            vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
        );
    }

    #[test]
    fn hex_decode_invalid_characters() {
        assert!(decode_hex("gg").is_none());
        assert!(decode_hex("0g").is_none());
        assert!(decode_hex("g0").is_none());
        assert!(decode_hex("!@").is_none());
    }

    #[test]
    fn hex_decode_odd_length() {
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("1").is_none());
    }

    #[test]
    fn verify_signature_matches() {
        let secret = "topsecret";
        let body = b"{\"x\":1}";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let tag = mac.finalize().into_bytes();
        let sig = format!("sha256={}", {
            let mut s = String::new();
            for b in tag {
                s.push_str(&format!("{:02x}", b));
            }
            s
        });
        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", sig.parse().unwrap());
        assert!(verify_signature(secret, &headers, body).is_ok());
    }

    #[test]
    fn verify_signature_missing_header() {
        let headers = HeaderMap::new();
        let result = verify_signature("secret", &headers, b"body");
        assert!(matches!(result, Err(VerifyError::MissingHeader)));
    }

    #[test]
    fn verify_signature_invalid_header_format() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", "invalid".parse().unwrap());
        let result = verify_signature("secret", &headers, b"body");
        assert!(matches!(result, Err(VerifyError::InvalidHeader)));
    }

    #[test]
    fn verify_signature_wrong_signature() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Hub-Signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        let result = verify_signature("secret", &headers, b"body");
        assert!(matches!(result, Err(VerifyError::Mismatch)));
    }

    #[test]
    fn verify_signature_invalid_hex() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", "sha256=invalid_hex".parse().unwrap());
        let result = verify_signature("", &headers, b"body");
        assert!(matches!(result, Err(VerifyError::InvalidHeader)));
    }

    #[test]
    fn verify_signature_different_secrets() {
        let secret1 = "secret1";
        let secret2 = "secret2";
        let body = b"test body";

        let mut mac = Hmac::<Sha256>::new_from_slice(secret1.as_bytes()).unwrap();
        mac.update(body);
        let tag = mac.finalize().into_bytes();
        let sig = format!("sha256={}", {
            let mut s = String::new();
            for b in tag {
                s.push_str(&format!("{:02x}", b));
            }
            s
        });

        let mut headers = HeaderMap::new();
        headers.insert("X-Hub-Signature-256", sig.parse().unwrap());

        // Should fail with different secret
        let result = verify_signature(secret2, &headers, body);
        assert!(matches!(result, Err(VerifyError::Mismatch)));
    }

    // Helper function to create a mock comment for testing
    fn create_mock_comment_with_body(body: Option<String>) -> Comment {
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "node_id": "MDEyOklzc3VlQ29tbWVudDE=",
            "body": body,
            "body_text": body.as_deref().unwrap_or(""),
            "body_html": body.as_deref().unwrap_or(""),
            "user": {
                "login": "testuser",
                "id": 1,
                "node_id": "MDQ6VXNlcjE=",
                "avatar_url": "https://github.com/images/error/testuser_happy.gif",
                "gravatar_id": "",
                "url": "https://api.github.com/users/testuser",
                "html_url": "https://github.com/testuser",
                "followers_url": "https://api.github.com/users/testuser/followers",
                "following_url": "https://api.github.com/users/testuser/following{/other_user}",
                "gists_url": "https://api.github.com/users/testuser/gists{/gist_id}",
                "starred_url": "https://api.github.com/users/testuser/starred{/owner}{/repo}",
                "subscriptions_url": "https://api.github.com/users/testuser/subscriptions",
                "organizations_url": "https://api.github.com/users/testuser/orgs",
                "repos_url": "https://api.github.com/users/testuser/repos",
                "events_url": "https://api.github.com/users/testuser/events{/privacy}",
                "received_events_url": "https://api.github.com/users/testuser/received_events",
                "type": "User",
                "site_admin": false
            },
            "created_at": "2023-01-01T00:00:00Z",
            "updated_at": "2023-01-01T00:00:00Z",
            "html_url": "https://github.com/owner/repo/issues/1#issuecomment-1",
            "url": "https://api.github.com/repos/owner/repo/issues/comments/1",
            "issue_url": "https://api.github.com/repos/owner/repo/issues/1",
            "author_association": "OWNER"
        }))
        .unwrap()
    }

    #[test]
    fn comment_has_marker_found() {
        let comment = create_mock_comment_with_body(Some(
            "Some text <!-- sampo-bot:changeset-check --> more text".to_string(),
        ));
        assert!(comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_not_found() {
        let comment = create_mock_comment_with_body(Some("Some text without marker".to_string()));
        assert!(!comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_none_body() {
        let comment = create_mock_comment_with_body(None);
        assert!(!comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_empty_body() {
        let comment = create_mock_comment_with_body(Some("".to_string()));
        assert!(!comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_case_sensitive() {
        let comment =
            create_mock_comment_with_body(Some("<!-- SAMPO-BOT:CHANGESET-CHECK -->".to_string()));
        assert!(!comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_partial_match() {
        let comment =
            create_mock_comment_with_body(Some("<!-- sampo-bot:changeset -->".to_string()));
        assert!(!comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_multiple_markers() {
        let comment = create_mock_comment_with_body(Some(
            "First <!-- sampo-bot:changeset-check --> Second <!-- sampo-bot:changeset-check -->"
                .to_string(),
        ));
        assert!(comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn comment_has_marker_whitespace_around() {
        let comment = create_mock_comment_with_body(Some(
            "   <!-- sampo-bot:changeset-check -->   ".to_string(),
        ));
        assert!(comment_has_marker(
            &comment,
            "<!-- sampo-bot:changeset-check -->"
        ));
    }

    #[test]
    fn verify_error_display() {
        let err = VerifyError::MissingHeader;
        assert_eq!(err.to_string(), "missing signature header");

        let err = VerifyError::InvalidHeader;
        assert_eq!(err.to_string(), "invalid signature header");

        let err = VerifyError::Mismatch;
        assert_eq!(err.to_string(), "signature mismatch");

        let err = VerifyError::Internal("test error".to_string());
        assert_eq!(err.to_string(), "internal: test error");
    }

    #[test]
    fn changeset_message_format() {
        const MARKER: &str = "<!-- sampo-bot:changeset-check -->";

        let with_changeset = format!(
            "{marker}\n## 🧭 Changeset detected\n\nMerging this PR will bump the version and include these changes in the next release.\n",
            marker = MARKER
        );
        assert!(with_changeset.contains(MARKER));
        assert!(with_changeset.contains("🧭 Changeset detected"));
        assert!(with_changeset.contains("bump the version"));

        let without_changeset = format!(
            "{marker}\n## ⚠️ No changeset detected\n\nIf this PR isn't meant to release a new version, no action needed. If it should, add a changeset to bump the version.\n",
            marker = MARKER
        );
        assert!(without_changeset.contains(MARKER));
        assert!(without_changeset.contains("⚠️ No changeset detected"));
        assert!(without_changeset.contains("add a changeset"));
    }
}
