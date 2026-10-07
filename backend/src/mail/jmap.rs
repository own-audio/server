// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mail over JMAP (RFC 8620/8621): plain HTTPS/JSON calls to a Stalwart
//! server. The hosted edition's original mailer, kept until it moves to SMTP.

use crate::app::config::MailConfig;
use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

fn basic_auth(user: &str, password: &str) -> String {
    format!("Basic {}", BASE64.encode(format!("{user}:{password}")))
}

struct JmapSession {
    api_url: String,
    account_id: String,
}

async fn get_session(client: &reqwest::Client, cfg: &MailConfig) -> anyhow::Result<JmapSession> {
    let base = cfg.jmap_base_url().unwrap_or_default().trim_end_matches('/');
    let resp = client
        .get(format!("{base}/.well-known/jmap"))
        .header("Authorization", basic_auth(cfg.jmap_user.as_deref().unwrap_or_default(), cfg.jmap_password.as_deref().unwrap_or_default()))
        .send()
        .await
        .context("jmap: session request failed")?;

    if !resp.status().is_success() {
        bail!("jmap: session request returned {}", resp.status());
    }

    let body: Value = resp.json().await.context("jmap: session response was not JSON")?;
    let api_url = body["apiUrl"]
        .as_str()
        .context("jmap: session response missing apiUrl")?
        .to_string();

    let primary = &body["primaryAccounts"];
    let account_id = primary
        .get("urn:ietf:params:jmap:submission")
        .or_else(|| primary.get("urn:ietf:params:jmap:mail"))
        .and_then(Value::as_str)
        .context("jmap: session has no mail account")?
        .to_string();

    Ok(JmapSession { api_url, account_id })
}

async fn jmap_call(
    client: &reqwest::Client,
    cfg: &MailConfig,
    session: &JmapSession,
    method_calls: Value,
) -> anyhow::Result<Value> {
    let resp = client
        .post(&session.api_url)
        .header("Authorization", basic_auth(cfg.jmap_user.as_deref().unwrap_or_default(), cfg.jmap_password.as_deref().unwrap_or_default()))
        .json(&json!({
            "using": [
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
                "urn:ietf:params:jmap:submission",
            ],
            "methodCalls": method_calls,
        }))
        .send()
        .await
        .context("jmap: api call failed")?;

    if !resp.status().is_success() {
        bail!("jmap: api call returned {}", resp.status());
    }

    resp.json::<Value>().await.context("jmap: api response was not JSON")
}

/// Pick the args of the first method response with the given name.
fn pick<'a>(responses: &'a Value, name: &str) -> Option<&'a Value> {
    responses.as_array()?.iter().find(|r| r[0].as_str() == Some(name)).map(|r| &r[1])
}

/// Send through the JMAP server in `cfg` (Email/set, then EmailSubmission/set).
pub(super) async fn send(cfg: &MailConfig, to: &str, subject: &str, text: &str, html: &str) -> anyhow::Result<()> {

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("jmap: failed to build http client")?;

    let session = get_session(&client, cfg).await?;

    let lookup = jmap_call(
        &client,
        cfg,
        &session,
        json!([
            ["Mailbox/query", {"accountId": session.account_id, "filter": {"role": "drafts"}}, "m1"],
            ["Identity/get", {"accountId": session.account_id, "ids": null}, "i1"],
        ]),
    )
    .await?;

    let method_responses = lookup["methodResponses"].clone();
    let drafts_mailbox_id = pick(&method_responses, "Mailbox/query")
        .and_then(|v| v["ids"][0].as_str())
        .context("jmap: no drafts mailbox found")?;

    let identities = pick(&method_responses, "Identity/get")
        .and_then(|v| v["list"].as_array())
        .cloned()
        .unwrap_or_default();
    let identity = identities
        .iter()
        .find(|i| i["email"].as_str() == Some(cfg.from_address.as_str()))
        .or_else(|| identities.first())
        .context("jmap: no submission identity available")?;
    let identity_id = identity["id"].as_str().context("jmap: identity missing id")?;

    let send = jmap_call(
        &client,
        cfg,
        &session,
        json!([
            ["Email/set", {
                "accountId": session.account_id,
                "create": {
                    "msg1": {
                        "mailboxIds": {drafts_mailbox_id: true},
                        "from": [{"email": cfg.from_address, "name": cfg.from_name}],
                        "to": [{"email": to}],
                        "subject": subject,
                        "bodyValues": {
                            "text_part": {"value": text, "charset": "utf-8"},
                            "html_part": {"value": html, "charset": "utf-8"},
                        },
                        "textBody": [{"partId": "text_part", "type": "text/plain"}],
                        "htmlBody": [{"partId": "html_part", "type": "text/html"}],
                    }
                }
            }, "e1"],
            ["EmailSubmission/set", {
                "accountId": session.account_id,
                "create": {
                    "sub1": {
                        "emailId": "#msg1",
                        "identityId": identity_id,
                        "envelope": {
                            "mailFrom": {"email": cfg.from_address},
                            "rcptTo": [{"email": to}],
                        },
                    }
                },
                "onSuccessDestroyEmail": ["#sub1"],
            }, "e2"],
        ]),
    )
    .await?;

    let send_responses = send["methodResponses"].clone();
    if let Some(not_created) = pick(&send_responses, "Email/set").and_then(|v| v["notCreated"]["msg1"].as_object())
    {
        bail!("jmap: Email/set failed: {not_created:?}");
    }
    if let Some(not_created) =
        pick(&send_responses, "EmailSubmission/set").and_then(|v| v["notCreated"]["sub1"].as_object())
    {
        bail!("jmap: EmailSubmission/set failed: {not_created:?}");
    }

    Ok(())
}
