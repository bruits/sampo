mod changeset;
mod error;

use crate::{
    changeset::analyze_pr_changesets,
    error::{BotError, Result, VerifyError},
};
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use octocrab::models::issues::Comment;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Clone)]
struct AppState {
    webhook_secret: Arc<String>,
    app_id: u64,
    private_key: Arc<EncodingKey>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: u64,
    exp: u64,
    iss: String,
}

#[tokio::main]
async fn main() -> Result<()> {
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

    // GitHub App configuration
    let app_id: u64 = std::env::var("GITHUB_APP_ID")
        .expect("GITHUB_APP_ID env var must be set")
        .parse()
        .expect("GITHUB_APP_ID must be a valid number");

    let private_key_pem = std::env::var("GITHUB_PRIVATE_KEY")
        .expect("GITHUB_PRIVATE_KEY env var must be set (PEM format)");

    let private_key =
        EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).expect("Invalid private key format");

    // Create octocrab instance (we'll update it per-request with installation tokens)
    let app_state = AppState {
        webhook_secret: Arc::new(secret),
        app_id,
        private_key: Arc::new(private_key),
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
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
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

    // Ignore PRs created by the Sampo GitHub Action (release PRs)
    // These PRs intentionally may not include a changeset.
    if is_sampo_action_release_pr(&payload) {
        return Ok((StatusCode::OK, "ignored release PR from sampo action"));
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

    let head_sha = payload
        .get("pull_request")
        .and_then(|pr| pr.get("head"))
        .and_then(|head| head.get("sha"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing PR head sha".to_string()))?
        .to_string();

    // Get installation token for this repository
    let installation_octo = match get_installation_client(&state, &owner, &repo).await {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to get installation token: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to authenticate with repository".into(),
            ));
        }
    };

    let analysis = match analyze_pr_changesets(
        &installation_octo,
        &owner,
        &repo,
        pr_number,
        &head_sha,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            error!("error analysing changesets: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to evaluate changesets".into(),
            ));
        }
    };

    if analysis.has_changeset {
        info!("changeset detected for PR #{}", pr_number);
    } else {
        info!("no valid changeset detected for PR #{}", pr_number);
    }

    const COMMENT_MARKER: &str = "<!-- sampo-bot:changeset-check -->";
    let existing_comment =
        match find_sticky_comment(&installation_octo, &owner, &repo, pr_number, COMMENT_MARKER)
            .await
        {
            Ok(comment) => comment,
            Err(e) => {
                error!("failed to locate existing comment: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to inspect existing comment".into(),
                ));
            }
        };

    let mut approval_state = existing_comment
        .as_ref()
        .and_then(|comment| comment.body.as_deref())
        .and_then(parse_approval_state)
        .unwrap_or_default();

    if analysis.has_changeset {
        if let Some(review_id) = approval_state.approval_review_id {
            match review_is_approved(&installation_octo, &owner, &repo, pr_number, review_id).await
            {
                Ok(true) => {}
                Ok(false) => {
                    approval_state.approval_review_id = None;
                }
                Err(e) => {
                    error!("failed to inspect existing review: {}", e);
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to inspect review state".into(),
                    ));
                }
            }
        }
        if approval_state.approval_review_id.is_none() {
            match submit_review(
                &installation_octo,
                &owner,
                &repo,
                pr_number,
                octocrab::models::pulls::ReviewAction::Approve,
                None,
            )
            .await
            {
                Ok(review) => {
                    approval_state.approval_review_id = Some(review.id.0);
                }
                Err(e) => {
                    error!("failed to submit review: {}", e);
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to submit review".into(),
                    ));
                }
            }
        }
        approval_state.approved_head = Some(head_sha.clone());
    } else {
        if let Some(review_id) = approval_state.approval_review_id
            && let Err(e) =
                dismiss_review(&installation_octo, &owner, &repo, pr_number, review_id).await
        {
            error!("failed to dismiss review: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to dismiss review".into(),
            ));
        }
        approval_state.approval_review_id = None;
        approval_state.approved_head = None;
    }

    let comment_body =
        match build_comment_body(COMMENT_MARKER, &analysis.comment_markdown, &approval_state) {
            Ok(body) => body,
            Err(e) => {
                error!("failed to build comment body: {}", e);
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to prepare comment".into(),
                ));
            }
        };

    if let Err(e) = upsert_sticky_comment(
        &installation_octo,
        &owner,
        &repo,
        pr_number,
        existing_comment.as_ref().map(|c| c.id),
        &comment_body,
    )
    .await
    {
        error!("failed to upsert comment: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to comment".into(),
        ));
    }

    Ok((StatusCode::OK, "ok"))
}

fn verify_signature(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> std::result::Result<(), VerifyError> {
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
    if !bytes.len().is_multiple_of(2) {
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

/// Detect whether the PR was generated by the Sampo GitHub Action (release PR).
/// These PRs include a standard sentence in the body.
fn is_sampo_action_release_pr(payload: &serde_json::Value) -> bool {
    let pr = match payload.get("pull_request") {
        Some(v) => v,
        None => return false,
    };

    let body = pr.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let head_ref = pr
        .get("head")
        .and_then(|h| h.get("ref"))
        .and_then(|r| r.as_str())
        .unwrap_or("");

    // Multiple heuristics to detect release PRs created by sampo-github-action:

    // 1. Check if PR body contains the signature phrase from sampo-github-action
    if body.contains("Sampo GitHub Action") {
        return true;
    }

    // 2. Check if branch name follows the release pattern used by sampo-github-action
    // Default branch is "release/sampo" but can be configured
    if head_ref.starts_with("release/") {
        return true;
    }

    false
}

async fn find_sticky_comment(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    marker: &str,
) -> Result<Option<Comment>> {
    let mut page = octo
        .issues(owner, repo)
        .list_comments(pr)
        .send()
        .await
        .map_err(BotError::from_comments)?;
    loop {
        for comment in &page {
            if comment_has_marker(comment, marker) {
                return Ok(Some(comment.clone()));
            }
        }
        if let Some(next) = octo
            .get_page::<Comment>(&page.next)
            .await
            .map_err(BotError::from_comments)?
        {
            page = next;
        } else {
            break;
        }
    }
    Ok(None)
}

async fn upsert_sticky_comment(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    existing: Option<octocrab::models::CommentId>,
    body: &str,
) -> Result<()> {
    if let Some(id) = existing {
        octo.issues(owner, repo)
            .update_comment(id, body)
            .await
            .map_err(BotError::from_comments)?;
    } else {
        octo.issues(owner, repo)
            .create_comment(pr, body)
            .await
            .map_err(BotError::from_comments)?;
    }
    Ok(())
}

async fn submit_review(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    action: octocrab::models::pulls::ReviewAction,
    body: Option<&str>,
) -> Result<octocrab::models::pulls::Review> {
    let route = format!(
        "/repos/{owner}/{repo}/pulls/{pr}/reviews",
        owner = owner,
        repo = repo,
        pr = pr
    );

    let mut payload = serde_json::Map::new();
    payload.insert(
        "event".to_string(),
        serde_json::to_value(action).map_err(|err| {
            BotError::Internal(format!("failed to serialize review action: {err}"))
        })?,
    );
    if let Some(text) = body {
        payload.insert(
            "body".to_string(),
            serde_json::Value::String(text.to_string()),
        );
    }
    let payload = serde_json::Value::Object(payload);

    octo.post::<serde_json::Value, octocrab::models::pulls::Review>(route, Some(&payload))
        .await
        .map_err(|err| BotError::Internal(format!("failed to submit review: {err}")))
}

async fn dismiss_review(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    review_id: u64,
) -> Result<()> {
    let route = format!(
        "/repos/{owner}/{repo}/pulls/{pr}/reviews/{review_id}/dismissals",
        owner = owner,
        repo = repo,
        pr = pr,
        review_id = review_id
    );
    let payload = serde_json::json!({
        "message": "Dismissed by Sampo GitHub Bot: changeset removed from PR.",
    });

    match octo
        .put::<serde_json::Value, _, _>(route, Some(&payload))
        .await
    {
        Ok(_) => Ok(()),
        Err(octocrab::Error::GitHub { source, .. })
            if matches!(
                source.status_code,
                StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(BotError::Internal(format!(
            "failed to dismiss review: {err}"
        ))),
    }
}

async fn review_is_approved(
    octo: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    pr: u64,
    review_id: u64,
) -> Result<bool> {
    let route = format!(
        "/repos/{owner}/{repo}/pulls/{pr}/reviews/{review_id}",
        owner = owner,
        repo = repo,
        pr = pr,
        review_id = review_id
    );
    match octo
        .get::<octocrab::models::pulls::Review, _, ()>(route, None)
        .await
    {
        Ok(review) => Ok(matches!(
            review.state,
            Some(octocrab::models::pulls::ReviewState::Approved)
        )),
        Err(octocrab::Error::GitHub { source, .. })
            if source.status_code == StatusCode::NOT_FOUND =>
        {
            Ok(false)
        }
        Err(err) => Err(BotError::Internal(format!(
            "failed to fetch review state: {err}"
        ))),
    }
}

fn build_comment_body(marker: &str, markdown: &str, state: &ApprovalState) -> Result<String> {
    let mut body = String::from(marker);
    body.push('\n');
    body.push_str(markdown);
    if !body.ends_with('\n') {
        body.push('\n');
    }

    let state_json = serde_json::to_string(state)
        .map_err(|err| BotError::Internal(format!("failed to serialize approval state: {err}")))?;
    body.push_str("<!-- sampo-bot:review-state ");
    body.push_str(&state_json);
    body.push_str(" -->");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Ok(body)
}

fn parse_approval_state(body: &str) -> Option<ApprovalState> {
    let marker = "<!-- sampo-bot:review-state ";
    let start = body.find(marker)?;
    let after_marker = &body[start + marker.len()..];
    let end = after_marker.find("-->")?;
    let json = after_marker[..end].trim();
    serde_json::from_str(json).ok()
}

#[derive(Default, Serialize, Deserialize)]
struct ApprovalState {
    approval_review_id: Option<u64>,
    approved_head: Option<String>,
}

fn comment_has_marker(c: &Comment, marker: &str) -> bool {
    c.body.as_deref().unwrap_or("").contains(marker)
}

/// Generate a JWT for GitHub App authentication
fn create_jwt(app_id: u64, private_key: &EncodingKey) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("SystemTime before UNIX_EPOCH")
        .as_secs();

    let claims = Claims {
        iat: now - 60,        // issued 60 seconds ago
        exp: now + (10 * 60), // expires in 10 minutes
        iss: app_id.to_string(),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("JWT".to_string());

    Ok(encode(&header, &claims, private_key)?)
}

/// Get installation ID for a repository
async fn get_installation_id(app_jwt: &str, owner: &str, repo: &str) -> Result<u64> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/{}/installation",
        owner, repo
    );

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", app_jwt))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "sampo-github-bot/1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(BotError::Internal(format!(
            "Failed to get installation: {}",
            response.status()
        )));
    }

    let installation: serde_json::Value = response.json().await?;
    installation
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| BotError::Internal("Installation ID not found".into()))
}

/// Get installation access token
async fn get_installation_token(app_jwt: &str, installation_id: u64) -> Result<String> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/app/installations/{}/access_tokens",
        installation_id
    );

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", app_jwt))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "sampo-github-bot/1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(BotError::Internal(format!(
            "Failed to get installation token: {}",
            response.status()
        )));
    }

    let token_response: serde_json::Value = response.json().await?;
    token_response
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| BotError::Internal("Installation token not found".into()))
}

/// Create an authenticated octocrab client for a specific repository
async fn get_installation_client(
    state: &AppState,
    owner: &str,
    repo: &str,
) -> Result<octocrab::Octocrab> {
    // Create JWT
    let jwt = create_jwt(state.app_id, &state.private_key)?;

    // Get installation ID for this repo
    let installation_id = get_installation_id(&jwt, owner, repo).await?;

    // Get installation token
    let installation_token = get_installation_token(&jwt, installation_id).await?;

    // Create authenticated octocrab client
    let client = octocrab::Octocrab::builder()
        .personal_token(installation_token)
        .build()
        .map_err(BotError::from_github_auth)?;

    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::VerifyError;

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
    fn approval_state_roundtrip() {
        let state = ApprovalState {
            approval_review_id: Some(99),
            approved_head: Some("deadbeef".to_string()),
        };
        let body = build_comment_body("<!-- sampo-bot:changeset-check -->", "body\n", &state)
            .expect("body builds");
        let parsed = parse_approval_state(&body).expect("state parses");
        assert_eq!(parsed.approval_review_id, state.approval_review_id);
        assert_eq!(parsed.approved_head, state.approved_head);
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
    fn release_pr_detection_by_body_phrase() {
        let payload = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "body": "This PR was generated by [Sampo GitHub Action](https://github.com/bruits/sampo/blob/main/crates/sampo-github-action/README.md).\n\n----\n\n...",
                "head": {"ref": "release/sampo"}
            }
        });
        assert!(is_sampo_action_release_pr(&payload));
    }

    #[test]
    fn non_release_pr_not_detected() {
        let payload = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "body": "Regular feature PR without changesets mention.",
                "head": {"ref": "feature/something"}
            }
        });
        assert!(!is_sampo_action_release_pr(&payload));
    }

    #[test]
    fn release_pr_detection_by_branch_name() {
        // Test detection by branch name even if body is modified
        let payload = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "body": "This body was manually edited and no longer contains the original text.",
                "head": {"ref": "release/sampo"}
            }
        });
        assert!(is_sampo_action_release_pr(&payload));
    }

    #[test]
    fn release_pr_detection_by_release_prefix() {
        // Test detection by any release/ prefix
        let payload = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "body": "Custom release PR",
                "head": {"ref": "release/custom-name"}
            }
        });
        assert!(is_sampo_action_release_pr(&payload));
    }

    #[test]
    fn release_pr_detection_robust_against_missing_body() {
        // Test detection when body is null but branch indicates release
        let payload = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "body": null,
                "head": {"ref": "release/sampo"}
            }
        });
        assert!(is_sampo_action_release_pr(&payload));
    }
}
