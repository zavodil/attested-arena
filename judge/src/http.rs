//! The enclave's way out: `wasi:http`, through the worker's allowlist.
//!
//! Every call keeps the exact bytes it sent and received. Hashes and signatures
//! later in the run are taken over those bytes, so nothing here parses,
//! normalises or re-encodes a body.
//!
//! Requests can be sent together ([`send_all`]): every request is handed to
//! the host first, then the answers are collected as they arrive, so twenty
//! model calls take about as long as the slowest one.
//!
//! Written on `wasi:http` directly rather than through a client crate: an
//! output stream takes at most what `check-write` grants (4096 bytes on the
//! worker), and the GPU evidence posted to NVIDIA is three times that, so the
//! body goes out in as many pieces as the stream asks for — after the request
//! is handed over, so the host is already draining it.

use wasi::http::outgoing_handler;
use wasi::http::types::{ErrorCode, Fields, IncomingBody, Method, OutgoingBody, OutgoingRequest, RequestOptions, Scheme};
use wasi::io::streams::{InputStream, OutputStream, StreamError};

const CONNECT_TIMEOUT_NS: u64 = 10_000_000_000;
const READ_CHUNK: u64 = 64 * 1024;

/// An answer as it arrived: status, headers (names lowercased) and body.
pub struct Reply {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, cut to what an error message can carry.
    pub fn excerpt(&self) -> String {
        String::from_utf8_lossy(&self.body).trim().chars().take(300).collect()
    }
}

pub fn get(url: &str, headers: &[(&str, &str)]) -> Result<Reply, String> {
    send(Method::Get, url, headers, None)
}

pub fn post(url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Reply, String> {
    send(Method::Post, url, headers, Some(body))
}

/// One request to send with others.
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// `Some` for a POST.
    pub body: Option<Vec<u8>>,
}

/// Send every request, then collect every answer: all are in flight at once.
/// The answers come back in the order of `requests`.
pub fn send_all(requests: &[Request]) -> Vec<Result<Reply, String>> {
    let mut pending: Vec<Option<Result<Pending, String>>> = requests
        .iter()
        .map(|r| {
            let headers: Vec<(&str, &str)> = r.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let method = if r.body.is_some() { Method::Post } else { Method::Get };
            Some(start(method, &r.url, &headers, r.body.as_deref()))
        })
        .collect();
    let mut replies: Vec<Option<Result<Reply, String>>> = (0..requests.len()).map(|_| None).collect();
    loop {
        let waiting: Vec<usize> = (0..pending.len()).filter(|&i| matches!(pending[i], Some(Ok(_)))).collect();
        for i in 0..pending.len() {
            if matches!(pending[i], Some(Err(_))) {
                if let Some(Err(e)) = pending[i].take() {
                    replies[i] = Some(Err(e));
                }
            }
        }
        if waiting.is_empty() {
            break;
        }
        let pollables: Vec<_> = waiting
            .iter()
            .map(|&i| match &pending[i] {
                Some(Ok(p)) => p.future.subscribe(),
                _ => unreachable!("only requests in flight are waited on"),
            })
            .collect();
        let refs: Vec<&wasi::io::poll::Pollable> = pollables.iter().collect();
        let ready = wasi::io::poll::poll(&refs);
        drop(refs);
        drop(pollables);
        for r in ready {
            let i = waiting[r as usize];
            if let Some(Ok(p)) = pending[i].take() {
                replies[i] = Some(finish(p));
            }
        }
    }
    replies.into_iter().map(|r| r.unwrap_or_else(|| Err("the request was never answered".to_string()))).collect()
}

/// A request handed to the host, its answer not yet collected.
struct Pending {
    host: String,
    future: wasi::http::types::FutureIncomingResponse,
}

fn send(method: Method, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Reply, String> {
    finish(start(method, url, headers, body)?)
}

fn start(method: Method, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Pending, String> {
    let host_name = host(url);
    let refused = |what: &str| format!("the request to {host_name} could not be built: {what}");
    let path = url
        .strip_prefix("https://")
        .ok_or_else(|| refused("only https URLs are used"))?
        .get(host_name.len()..)
        .filter(|p| !p.is_empty())
        .unwrap_or("/");

    let fields = Fields::new();
    for (name, value) in headers {
        fields.append(&name.to_string(), &value.as_bytes().to_vec()).map_err(|e| refused(&format!("header {name}: {e:?}")))?;
    }
    if let Some(body) = body {
        fields.append(&"Content-Length".to_string(), &body.len().to_string().into_bytes()).map_err(|e| refused(&format!("{e:?}")))?;
    }
    let request = OutgoingRequest::new(fields);
    request.set_method(&method).map_err(|()| refused("method"))?;
    request.set_scheme(Some(&Scheme::Https)).map_err(|()| refused("scheme"))?;
    request.set_authority(Some(host_name)).map_err(|()| refused("authority"))?;
    request.set_path_with_query(Some(if path.starts_with('/') { path } else { "/" })).map_err(|()| refused("path"))?;
    let outgoing = request.body().map_err(|()| refused("body"))?;
    let options = RequestOptions::new();
    options.set_connect_timeout(Some(CONNECT_TIMEOUT_NS)).map_err(|()| refused("timeout"))?;

    let pending = outgoing_handler::handle(request, Some(options)).map_err(|e| failed(host_name, &e))?;
    if let Some(body) = body {
        let stream = outgoing.write().map_err(|()| refused("body stream"))?;
        write_all(&stream, body).map_err(|e| format!("sending the request to {host_name} failed: {e}"))?;
    }
    OutgoingBody::finish(outgoing, None).map_err(|e| failed(host_name, &e))?;
    Ok(Pending { host: host_name.to_string(), future: pending })
}

fn finish(pending: Pending) -> Result<Reply, String> {
    let host_name = pending.host.as_str();
    let refused = |what: &str| format!("the answer from {host_name} could not be read: {what}");
    let response = loop {
        match pending.future.get() {
            Some(result) => break result.map_err(|()| refused("the response was already taken"))?.map_err(|e| failed(host_name, &e))?,
            None => pending.future.subscribe().block(),
        }
    };
    let status = response.status();
    let headers = response
        .headers()
        .entries()
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), String::from_utf8_lossy(&v).into_owned()))
        .collect();
    let incoming = response.consume().map_err(|()| refused("the response body was already taken"))?;
    let body = {
        let stream = incoming.stream().map_err(|()| refused("the response body stream"))?;
        read_all(&stream).map_err(|e| format!("the answer from {host_name} did not arrive whole: {e}"))?
    };
    drop(IncomingBody::finish(incoming));
    Ok(Reply { status, headers, body })
}

/// Write `bytes` in the pieces the stream grants, then wait for the flush.
fn write_all(stream: &OutputStream, mut bytes: &[u8]) -> Result<(), String> {
    while !bytes.is_empty() {
        let granted = stream.check_write().map_err(stream_error)?;
        if granted == 0 {
            stream.subscribe().block();
            continue;
        }
        let n = bytes.len().min(granted as usize);
        stream.write(&bytes[..n]).map_err(stream_error)?;
        bytes = &bytes[n..];
    }
    stream.blocking_flush().map_err(stream_error)
}

fn read_all(stream: &InputStream) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        match stream.blocking_read(READ_CHUNK) {
            Ok(chunk) => body.extend_from_slice(&chunk),
            Err(StreamError::Closed) => return Ok(body),
            Err(e) => return Err(stream_error(e)),
        }
    }
}

fn stream_error(e: StreamError) -> String {
    match e {
        StreamError::Closed => "the stream closed early".to_string(),
        StreamError::LastOperationFailed(err) => err.to_debug_string(),
    }
}

/// The host's refusal, said so that a timeout reads as one.
fn failed(host_name: &str, e: &ErrorCode) -> String {
    match e {
        ErrorCode::ConnectionTimeout | ErrorCode::ConnectionReadTimeout | ErrorCode::HttpResponseTimeout => format!(
            "{host_name} did not answer in time ({e:?}): the manifest gives each request to NEAR AI 120 s, and to any other host 30 s"
        ),
        _ => format!("{host_name} could not be reached: {e:?}"),
    }
}

/// The host of a URL, for messages: the path may carry query values nobody
/// needs to read in an error.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Percent-encode a query or path value: everything outside the unreserved
/// characters, byte by byte.
pub fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Percent-decoding, as Intel PCS encodes the certificate chains it puts in
/// headers.
pub fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = value.get(i + 1..i + 3).ok_or("a percent escape is cut short")?;
            out.push(u8::from_str_radix(pair, 16).map_err(|_| format!("`%{pair}` is not a percent escape"))?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "a decoded header is not UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_named_without_its_path_or_query() {
        assert_eq!(host("https://cloud-api.near.ai/v1/signature/x?model=m"), "cloud-api.near.ai");
        assert_eq!(host("https://api.trustedservices.intel.com?x"), "api.trustedservices.intel.com");
    }

    #[test]
    fn a_model_id_is_encoded_for_a_query() {
        assert_eq!(urlencode("Qwen/Qwen3.8-27B"), "Qwen%2FQwen3.8-27B");
    }

    #[test]
    fn intel_header_chains_decode() {
        assert_eq!(
            percent_decode("-----BEGIN%20CERTIFICATE-----%0AMIIC").unwrap(),
            "-----BEGIN CERTIFICATE-----\nMIIC"
        );
        assert!(percent_decode("%4").is_err());
        assert!(percent_decode("%zz").is_err());
    }
}
