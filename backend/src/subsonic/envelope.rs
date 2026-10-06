// SPDX-License-Identifier: AGPL-3.0-or-later
/// Shared `subsonic-response` envelope builder.
///
/// Every endpoint hands this a `serde_json::Value::Object` describing its
/// payload (scalars become XML attributes / JSON fields on the enclosing
/// element, nested objects/arrays become child elements) and this module
/// renders it as either XML (default) or JSON, per the `f` query param —
/// exactly the tree-shape correspondence the OpenSubsonic spec itself
/// assumes between its XML and JSON representations.
use axum::http::header;
use axum::response::{IntoResponse, Response};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::writer::Writer;
use serde_json::{Map, Value, json};
use std::io::Cursor;

const API_VERSION: &str = "1.16.1";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Xml,
    Json,
    Jsonp,
}

impl ResponseFormat {
    pub fn from_param(f: Option<&str>) -> Self {
        match f {
            Some("json") => ResponseFormat::Json,
            Some("jsonp") => ResponseFormat::Jsonp,
            _ => ResponseFormat::Xml,
        }
    }
}

/// Error codes from the Subsonic API spec (§ error handling).
#[derive(Debug, Clone, Copy)]
pub enum SubsonicErrorCode {
    Generic = 0,
    MissingParam = 10,
    ClientMustUpgrade = 20,
    ServerMustUpgrade = 30,
    WrongCredentials = 40,
    NotAuthorized = 50,
    NotFound = 70,
}

impl SubsonicErrorCode {
    fn default_message(self) -> &'static str {
        match self {
            SubsonicErrorCode::Generic => "A generic error occurred",
            SubsonicErrorCode::MissingParam => "Required parameter is missing",
            SubsonicErrorCode::ClientMustUpgrade => "Incompatible client version",
            SubsonicErrorCode::ServerMustUpgrade => "Incompatible server version",
            SubsonicErrorCode::WrongCredentials => "Wrong username or password",
            SubsonicErrorCode::NotAuthorized => "User is not authorized for the given operation",
            SubsonicErrorCode::NotFound => "The requested data was not found",
        }
    }
}

/// Build a successful envelope. `body` should be a JSON object whose keys
/// are the endpoint-specific payload (e.g. `{"artists": {...}}`), or an
/// empty object for endpoints with no extra payload (e.g. `ping`).
pub fn ok(format: ResponseFormat, jsonp_callback: Option<&str>, body: Value) -> Response {
    render(format, jsonp_callback, "ok", body)
}

pub fn error(
    format: ResponseFormat,
    jsonp_callback: Option<&str>,
    code: SubsonicErrorCode,
    message: Option<&str>,
) -> Response {
    let body = json!({
        "error": {
            "code": code as i32,
            "message": message.unwrap_or(code.default_message()),
        }
    });
    render(format, jsonp_callback, "failed", body)
}

fn render(format: ResponseFormat, jsonp_callback: Option<&str>, status: &str, body: Value) -> Response {
    let mut root = Map::new();
    root.insert("status".into(), json!(status));
    root.insert("version".into(), json!(API_VERSION));
    root.insert("type".into(), json!("audio2"));
    root.insert("serverVersion".into(), json!(SERVER_VERSION));
    root.insert("openSubsonic".into(), json!(true));
    if let Value::Object(map) = body {
        root.extend(map);
    }
    let root = Value::Object(root);

    match format {
        ResponseFormat::Xml => {
            let xml = render_xml(&root);
            ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], xml).into_response()
        }
        ResponseFormat::Json => {
            let payload = json!({ "subsonic-response": root });
            ([(header::CONTENT_TYPE, "application/json")], payload.to_string()).into_response()
        }
        ResponseFormat::Jsonp => {
            let payload = json!({ "subsonic-response": root });
            let callback = jsonp_callback.unwrap_or("callback");
            let body = format!("{callback}({payload})");
            ([(header::CONTENT_TYPE, "application/javascript")], body).into_response()
        }
    }
}

/// Every Subsonic server since the original emits the XML declaration, and
/// the protocol's own schema is served as a document that has one. Strict
/// parsers on the client side treat a bare root element as a malformed
/// document rather than as an encoding-less one.
const XML_DECLARATION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

fn render_xml(root: &Value) -> String {
    // The namespace is part of the XML representation only; the JSON one has
    // no equivalent and must not carry an "xmlns" field.
    let mut root = root.clone();
    if let Value::Object(map) = &mut root {
        map.insert("xmlns".into(), json!("http://subsonic.org/restapi"));
    }
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    write_element(&mut writer, "subsonic-response", &root);
    let body = String::from_utf8(writer.into_inner().into_inner()).unwrap_or_default();
    format!("{XML_DECLARATION}{body}")
}

/// The one key whose XML form is the element's text rather than an attribute.
///
/// The two representations genuinely diverge here: a genre is
/// `{"value": "Rock"}` in JSON but `<genre>Rock</genre>` in XML, and the same
/// holds for `lyrics` and for each `line` of a structured lyric. Rendered as an
/// attribute — which is what every other scalar becomes — an XML client reads
/// the genre name as empty.
const TEXT_CONTENT_KEY: &str = "value";

fn write_element(writer: &mut Writer<Cursor<Vec<u8>>>, tag: &str, value: &Value) {
    let Value::Object(map) = value else {
        return;
    };

    let mut start = BytesStart::new(tag);
    let mut children: Vec<(&str, &Value)> = Vec::new();
    let mut text: Option<String> = None;

    for (key, val) in map {
        match val {
            Value::Object(_) => children.push((key.as_str(), val)),
            Value::Array(items) => {
                for item in items {
                    children.push((key.as_str(), item));
                }
            }
            Value::Null => {}
            scalar if key == TEXT_CONTENT_KEY => text = Some(scalar_to_attr(scalar)),
            scalar => start.push_attribute((key.as_str(), scalar_to_attr(scalar).as_str())),
        }
    }

    if children.is_empty() && text.is_none() {
        let _ = writer.write_event(Event::Empty(start));
        return;
    }

    let _ = writer.write_event(Event::Start(start));
    if let Some(text) = text {
        let _ = writer.write_event(Event::Text(BytesText::new(&text)));
    }
    for (child_tag, child_val) in children {
        match child_val {
            Value::Object(_) => write_element(writer, child_tag, child_val),
            // A bare scalar array item (not used by any current endpoint,
            // handled defensively) — emit as a text-only element.
            other => {
                let _ = writer.write_event(Event::Start(BytesStart::new(child_tag)));
                let _ = writer.write_event(Event::Text(BytesText::new(&scalar_to_attr(other))));
                let _ = writer.write_event(Event::End(BytesEnd::new(child_tag)));
            }
        }
    }
    let _ = writer.write_event(Event::End(BytesEnd::new(tag)));
}

fn scalar_to_attr(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A genre's name is the element's text in XML but a `value` field in
    /// JSON. Rendering it as an attribute leaves XML clients with a blank name.
    #[test]
    fn renders_the_value_key_as_element_text_not_an_attribute() {
        let xml = render_xml(&json!({
            "genres": { "genre": [{ "value": "Rock", "songCount": 28 }] }
        }));
        assert!(xml.contains("<genre songCount=\"28\">Rock</genre>"), "got: {xml}");
    }

    #[test]
    fn other_scalars_stay_attributes() {
        let xml = render_xml(&json!({ "album": { "name": "Rumours", "songCount": 11 } }));
        assert!(xml.contains("name=\"Rumours\""), "got: {xml}");
        assert!(xml.contains("songCount=\"11\""), "got: {xml}");
    }

    /// An element carrying only `value` still needs an open/close pair; a
    /// self-closing tag would drop the text entirely.
    #[test]
    fn a_text_only_element_is_not_self_closed() {
        let xml = render_xml(&json!({ "lyrics": { "value": "Closing time" } }));
        assert!(xml.contains("<lyrics>Closing time</lyrics>"), "got: {xml}");
    }
}
