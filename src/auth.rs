use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use reqwest::{Client, redirect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use url::Url;

use crate::secrets::{self, OAuthCredentials};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const CALLBACK: &str = "http://localhost:1455/auth/callback";

struct CallbackListeners {
    ipv4: TcpListener,
    ipv6: Option<TcpListener>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub signed_in: bool,
    pub email: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

pub fn status() -> Result<AuthStatus, String> {
    let credentials = secrets::load_oauth()?;
    Ok(AuthStatus {
        signed_in: credentials.is_some(),
        email: credentials.and_then(|value| value.email),
    })
}

pub async fn start_login() -> Result<
    (
        String,
        tokio::sync::oneshot::Receiver<Result<OAuthCredentials, String>>,
    ),
    String,
> {
    let listeners = bind_callback_listeners(1455)
        .map_err(|_| "Port 1455 is in use. Close another Codex login and try again.".to_owned())?;

    let verifier = random_url_safe(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_url_safe(32);
    let auth_url = Url::parse_with_params(
        &format!("{ISSUER}/oauth/authorize"),
        [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", CALLBACK),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", &state),
            ("originator", "unfold"),
        ],
    )
    .map_err(|error| format!("Could not create the authorization URL: {error}"))?;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = receive_callback(listeners, &state, &verifier).await;
        let _ = sender.send(result);
    });

    Ok((auth_url.to_string(), receiver))
}

pub async fn valid_access_token() -> Result<(String, Option<String>), String> {
    let mut credentials = load_credentials()?;
    let now = now_ms()?;
    if credentials.expires_at_ms > now + 30_000 {
        return Ok((credentials.access_token, credentials.account_id));
    }

    refresh_credentials(&mut credentials, now).await?;
    Ok((credentials.access_token, credentials.account_id))
}

fn load_credentials() -> Result<OAuthCredentials, String> {
    secrets::load_oauth()?
        .ok_or_else(|| "Sign in to ChatGPT before generating an example".to_owned())
}

async fn refresh_credentials(credentials: &mut OAuthCredentials, now: u64) -> Result<(), String> {
    let tokens = request_tokens(
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", credentials.refresh_token.as_str()),
            ("client_id", CLIENT_ID),
        ],
        "Could not refresh ChatGPT sign-in",
        "ChatGPT sign-in has expired",
    )
    .await?;

    update_credentials(credentials, tokens, now);
    secrets::save_oauth(credentials)
}

async fn request_tokens(
    form: &[(&str, &str)],
    send_error: &str,
    status_error: &str,
) -> Result<TokenResponse, String> {
    auth_client()?
        .post(format!("{ISSUER}/oauth/token"))
        .form(form)
        .send()
        .await
        .map_err(|error| format!("{send_error}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{status_error}: {error}"))?
        .json()
        .await
        .map_err(|error| format!("ChatGPT returned an invalid token response: {error}"))
}

fn update_credentials(credentials: &mut OAuthCredentials, tokens: TokenResponse, now: u64) {
    credentials.access_token = tokens.access_token;
    credentials.refresh_token = tokens.refresh_token;
    credentials.expires_at_ms = now + tokens.expires_in.unwrap_or(3600) * 1000;
    if let Some(id_token) = tokens.id_token {
        let claims = jwt_claims(&id_token);
        credentials.account_id = account_id(&claims).or(credentials.account_id.take());
        credentials.email = claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(credentials.email.take());
    }
}

async fn receive_callback(
    listeners: CallbackListeners,
    expected_state: &str,
    verifier: &str,
) -> Result<OAuthCredentials, String> {
    let mut stream = accept_callback(listeners).await?;
    let request = read_callback(&mut stream).await?;
    let result = process_callback(&request, expected_state, verifier).await;

    write_callback_response(&mut stream, &result).await;
    result
}

async fn accept_callback(listeners: CallbackListeners) -> Result<tokio::net::TcpStream, String> {
    let accept = async {
        match listeners.ipv6 {
            Some(ipv6) => {
                tokio::select! {
                    result = listeners.ipv4.accept() => result,
                    result = ipv6.accept() => result,
                }
            }
            None => listeners.ipv4.accept().await,
        }
    };
    let (stream, _) = timeout(Duration::from_secs(300), accept)
        .await
        .map_err(|_| "ChatGPT login timed out".to_owned())?
        .map_err(|error| format!("Could not accept the login callback: {error}"))?;
    Ok(stream)
}

async fn read_callback(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    let mut buffer = vec![0_u8; 16 * 1024];
    let size = timeout(Duration::from_secs(10), stream.read(&mut buffer))
        .await
        .map_err(|_| "ChatGPT callback timed out".to_owned())?
        .map_err(|error| format!("Could not read the login callback: {error}"))?;
    Ok(String::from_utf8_lossy(&buffer[..size]).into_owned())
}

async fn process_callback(
    request: &str,
    expected_state: &str,
    verifier: &str,
) -> Result<OAuthCredentials, String> {
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "ChatGPT returned a malformed callback".to_owned())?;
    let callback = Url::parse(&format!("http://localhost{target}"))
        .map_err(|_| "ChatGPT returned a malformed callback URL".to_owned())?;

    if callback.path() != "/auth/callback" {
        return Err("Unexpected login callback path".to_owned());
    }
    if let Some(error) = callback
        .query_pairs()
        .find(|(key, _)| key == "error_description" || key == "error")
        .map(|(_, value)| value.into_owned())
    {
        return Err(error);
    }
    if callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value != expected_state)
        .unwrap_or(true)
    {
        return Err("Login state did not match; authorization was rejected".to_owned());
    }
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "ChatGPT did not return an authorization code".to_owned())?;
    exchange_code(&code, verifier).await
}

async fn write_callback_response(
    stream: &mut tokio::net::TcpStream,
    result: &Result<OAuthCredentials, String>,
) {
    let (status, title, detail) = match &result {
        Ok(_) => ("200 OK", "Sign-in complete", "You can return to Unfold."),
        Err(error) => ("400 Bad Request", "Sign-in failed", error.as_str()),
    };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title><style>body{{font-family:system-ui;max-width:42rem;margin:12vh auto;padding:2rem;background:#f3efe6;color:#1d2825}}h1{{font-family:Georgia,serif}}</style><h1>{title}</h1><p>{}</p>",
        html_escape(detail)
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}

fn bind_callback_listeners(port: u16) -> std::io::Result<CallbackListeners> {
    let ipv4 = bind_loopback(
        Domain::IPV4,
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)),
    )?;
    let ipv6 = bind_loopback(
        Domain::IPV6,
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0)),
    )
    .ok();
    Ok(CallbackListeners { ipv4, ipv6 })
}

fn bind_loopback(domain: Domain, address: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if domain == Domain::IPV6 {
        socket.set_only_v6(true)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(8)?;
    TcpListener::from_std(socket.into())
}

async fn exchange_code(code: &str, verifier: &str) -> Result<OAuthCredentials, String> {
    let tokens = request_tokens(
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", CALLBACK),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ],
        "Could not exchange the authorization code",
        "ChatGPT rejected the authorization code",
    )
    .await?;
    let claims = tokens
        .id_token
        .as_deref()
        .map(jwt_claims)
        .unwrap_or(Value::Null);

    Ok(OAuthCredentials {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at_ms: now_ms()? + tokens.expires_in.unwrap_or(3600) * 1000,
        account_id: account_id(&claims),
        email: claims
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn auth_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not configure ChatGPT sign-in: {error}"))
}

fn random_url_safe(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::rng().fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn jwt_claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

fn account_id(claims: &Value) -> Option<String> {
    claims
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            claims
                .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            claims
                .pointer("/organizations/0/id")
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|_| "System clock is before the Unix epoch".to_owned())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_invalid_callback_requests_before_token_exchange() {
        let cases = [
            ("garbage", "ChatGPT returned a malformed callback"),
            ("GET /wrong HTTP/1.1", "Unexpected login callback path"),
            ("GET /auth/callback?error=denied HTTP/1.1", "denied"),
            (
                "GET /auth/callback?state=wrong HTTP/1.1",
                "Login state did not match",
            ),
            (
                "GET /auth/callback?state=expected HTTP/1.1",
                "did not return an authorization code",
            ),
        ];
        for (request, expected) in cases {
            let error = process_callback(request, "expected", "verifier")
                .await
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }
    use tokio::net::TcpStream;

    #[test]
    fn extracts_nested_chatgpt_account_id() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "account-123" }
        });
        assert_eq!(account_id(&claims), Some("account-123".to_owned()));
    }

    #[test]
    fn escapes_callback_error_html() {
        assert_eq!(html_escape("<bad & worse>"), "&lt;bad &amp; worse&gt;");
    }

    #[test]
    fn refreshed_tokens_update_claims_and_expiry() {
        let claims = URL_SAFE_NO_PAD.encode(br#"{"email":"new@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"new-account"}}"#);
        let mut credentials = OAuthCredentials {
            access_token: "old-access".to_owned(),
            refresh_token: "old-refresh".to_owned(),
            expires_at_ms: 0,
            account_id: Some("old-account".to_owned()),
            email: Some("old@example.com".to_owned()),
        };

        update_credentials(
            &mut credentials,
            TokenResponse {
                access_token: "new-access".to_owned(),
                refresh_token: "new-refresh".to_owned(),
                id_token: Some(format!("header.{claims}.signature")),
                expires_in: Some(120),
            },
            1_000,
        );

        assert_eq!(credentials.access_token, "new-access");
        assert_eq!(credentials.refresh_token, "new-refresh");
        assert_eq!(credentials.expires_at_ms, 121_000);
        assert_eq!(credentials.account_id.as_deref(), Some("new-account"));
        assert_eq!(credentials.email.as_deref(), Some("new@example.com"));
    }

    #[tokio::test]
    async fn callback_listens_on_ipv4_and_ipv6_loopback() {
        let probe = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let listeners = bind_callback_listeners(port).unwrap();

        let ipv4_connect = TcpStream::connect((Ipv4Addr::LOCALHOST, port));
        let (ipv4_client, ipv4_accept) = tokio::join!(ipv4_connect, listeners.ipv4.accept());
        assert!(ipv4_client.is_ok());
        assert!(ipv4_accept.is_ok());

        if let Some(ipv6) = listeners.ipv6 {
            let ipv6_connect = TcpStream::connect((Ipv6Addr::LOCALHOST, port));
            let (ipv6_client, ipv6_accept) = tokio::join!(ipv6_connect, ipv6.accept());
            assert!(ipv6_client.is_ok());
            assert!(ipv6_accept.is_ok());
        }
    }
}
