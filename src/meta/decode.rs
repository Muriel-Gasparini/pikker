use crate::runtime::IntegrationError;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

fn invalid() -> IntegrationError {
    IntegrationError::Invalid("Formato OP.GG não reconhecido")
}
const LIMIT: usize = 2 * 1024 * 1024;

pub fn rpc_response(body: &str, id: u64) -> Result<Value, IntegrationError> {
    if body.len() > LIMIT {
        return Err(invalid());
    }
    let frames: Vec<Value> = if body.trim_start().starts_with('{') {
        vec![serde_json::from_str(body)?]
    } else {
        let mut frames = Vec::new();
        let mut data = String::new();
        for line in body.lines().chain(std::iter::once("")) {
            if line.is_empty() {
                if !data.is_empty() {
                    frames.push(serde_json::from_str(&data)?);
                    data.clear();
                }
            } else if let Some(line) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(line.trim_start());
            }
        }
        frames
    };
    let frame = frames
        .into_iter()
        .find(|f| f.get("id").and_then(Value::as_u64) == Some(id))
        .ok_or_else(invalid)?;
    if frame.get("error").is_some() {
        return Err(invalid());
    }
    frame.get("result").cloned().ok_or_else(invalid)
}

pub fn decode_envelope(body: &str, id: u64) -> Result<Value, IntegrationError> {
    let result = rpc_response(body, id)?;
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(invalid());
    }
    if let Some(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let text = content
        .iter()
        .find_map(|v| {
            (v.get("type")?.as_str()? == "text")
                .then(|| v.get("text")?.as_str())
                .flatten()
        })
        .ok_or_else(invalid)?;
    decode_opgg(text)
}

pub fn decode_opgg(text: &str) -> Result<Value, IntegrationError> {
    if text.len() > LIMIT {
        return Err(invalid());
    }
    if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
        return Ok(serde_json::from_str(text)?);
    }
    let (headers, expression) = text.split_once("\n\n").ok_or_else(invalid)?;
    let mut schemas = BTreeMap::new();
    for line in headers.lines() {
        let (name, fields) = line
            .strip_prefix("class ")
            .and_then(|s| s.split_once(':'))
            .ok_or_else(invalid)?;
        if !identifier(name) {
            return Err(invalid());
        }
        let fields: Vec<_> = fields
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect();
        let mut unique = std::collections::BTreeSet::new();
        if fields.is_empty()
            || fields.iter().any(|f| !identifier(f) || !unique.insert(f))
            || schemas.insert(name.to_owned(), fields).is_some()
        {
            return Err(invalid());
        }
    }
    let mut parser = Parser {
        text: expression.as_bytes(),
        pos: 0,
        schemas,
    };
    let value = parser.value(0)?;
    parser.space();
    if parser.pos != parser.text.len() {
        return Err(invalid());
    }
    Ok(value)
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && (s.as_bytes()[0].is_ascii_alphabetic() || s.as_bytes()[0] == b'_')
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
    schemas: BTreeMap<String, Vec<String>>,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.text.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }
    fn consume(&mut self, c: u8) -> bool {
        self.space();
        if self.text.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, IntegrationError> {
        if depth > 40 {
            return Err(invalid());
        }
        self.space();
        match self.text.get(self.pos).copied().ok_or_else(invalid)? {
            b'"' => {
                let start = self.pos;
                self.pos += 1;
                loop {
                    match self.text.get(self.pos).ok_or_else(invalid)? {
                        b'\\' => self.pos += 2,
                        b'"' => {
                            self.pos += 1;
                            break;
                        }
                        _ => self.pos += 1,
                    }
                }
                Ok(serde_json::from_slice(&self.text[start..self.pos])?)
            }
            b'[' => {
                self.pos += 1;
                Ok(Value::Array(self.arguments(b']', depth)?))
            }
            b'-' | b'0'..=b'9' => {
                let start = self.pos;
                while self
                    .text
                    .get(self.pos)
                    .is_some_and(|b| b.is_ascii_digit() || b"-.eE+".contains(b))
                {
                    self.pos += 1;
                }
                Ok(serde_json::from_slice(&self.text[start..self.pos])?)
            }
            _ => {
                let start = self.pos;
                while self
                    .text
                    .get(self.pos)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    self.pos += 1;
                }
                if start == self.pos {
                    return Err(invalid());
                }
                let name =
                    std::str::from_utf8(&self.text[start..self.pos]).map_err(|_| invalid())?;
                match name {
                    "null" | "None" => return Ok(Value::Null),
                    "true" | "True" => return Ok(Value::Bool(true)),
                    "false" | "False" => return Ok(Value::Bool(false)),
                    _ => {}
                }
                let fields = self.schemas.get(name).cloned().ok_or_else(invalid)?;
                if !self.consume(b'(') {
                    return Err(invalid());
                }
                let values = self.arguments(b')', depth)?;
                if values.len() != fields.len() {
                    return Err(invalid());
                }
                Ok(Value::Object(
                    fields.into_iter().zip(values).collect::<Map<_, _>>(),
                ))
            }
        }
    }
    fn arguments(&mut self, end: u8, depth: usize) -> Result<Vec<Value>, IntegrationError> {
        let mut values = Vec::new();
        if self.consume(end) {
            return Ok(values);
        }
        loop {
            values.push(self.value(depth + 1)?);
            if self.consume(end) {
                return Ok(values);
            }
            if !self.consume(b',') {
                return Err(invalid());
            }
        }
    }
}
