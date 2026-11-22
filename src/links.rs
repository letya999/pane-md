use std::{
    path::{Path, PathBuf},
    string::FromUtf8Error,
};

pub fn markdown_path(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        ["md", "markdown", "mdown", "mkd"]
            .iter()
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
    External(String),
    Anchor(String),
    Local { path: PathBuf, anchor: String },
}

#[derive(Debug)]
pub enum TargetError {
    Decode(DecodeError),
    Invalid,
    Unsupported,
}

pub fn destination(raw: &str) -> Result<Destination, TargetError> {
    if raw.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(TargetError::Invalid);
    }
    if let Some(fragment) = raw.strip_prefix('#') {
        return Ok(Destination::Anchor(
            decode_percent(fragment).map_err(TargetError::Decode)?,
        ));
    }
    if let Some(host) = raw.strip_prefix("//") {
        if host.split(['/', '?', '#']).next().is_none_or(str::is_empty) {
            return Err(TargetError::Invalid);
        }
        return Ok(Destination::External(format!("https:{raw}")));
    }
    if let Some((scheme, rest)) = raw.split_once(':')
        && is_scheme(scheme)
        && !(cfg!(windows) && scheme.len() == 1 && rest.starts_with(['/', '\\']))
    {
        let scheme = scheme.to_ascii_lowercase();
        return match scheme.as_str() {
            "file" => file_uri(rest),
            "https" | "http" | "ftp"
                if rest.strip_prefix("//").is_some_and(|address| {
                    address
                        .split(['/', '?', '#'])
                        .next()
                        .is_some_and(|host| !host.is_empty())
                }) =>
            {
                Ok(Destination::External(raw.to_owned()))
            }
            "mailto" | "tel" | "sms" if !rest.is_empty() => {
                Ok(Destination::External(raw.to_owned()))
            }
            "http" | "https" | "ftp" | "mailto" | "tel" | "sms" => Err(TargetError::Invalid),
            _ => Err(TargetError::Unsupported),
        };
    }
    local(raw)
}

fn is_scheme(scheme: &str) -> bool {
    scheme
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+.-".contains(&byte))
}

fn local(raw: &str) -> Result<Destination, TargetError> {
    let (path, anchor) = raw.split_once('#').unwrap_or((raw, ""));
    local_parts(
        decode_percent(path).map_err(TargetError::Decode)?,
        decode_percent(anchor).map_err(TargetError::Decode)?,
    )
}

fn local_parts(path: String, anchor: String) -> Result<Destination, TargetError> {
    if path.is_empty() {
        return Err(TargetError::Invalid);
    }
    if cfg!(windows) {
        let name = path.replace('/', "\\");
        let rest = if name.as_bytes().get(1) == Some(&b':') {
            if !name.as_bytes()[0].is_ascii_alphabetic() || !name[2..].starts_with('\\') {
                return Err(TargetError::Invalid);
            }
            &name[2..]
        } else {
            &name
        };
        if rest.starts_with("\\\\?\\") || rest.starts_with("\\\\.\\") || rest.contains(':') {
            return Err(TargetError::Unsupported);
        }
    }
    Ok(Destination::Local {
        path: PathBuf::from(path),
        anchor,
    })
}

fn file_uri(rest: &str) -> Result<Destination, TargetError> {
    let (rest, anchor) = rest.split_once('#').unwrap_or((rest, ""));
    let anchor = decode_percent(anchor).map_err(TargetError::Decode)?;
    let path = if let Some(after_slashes) = rest.strip_prefix("//")
        && !after_slashes.starts_with('/')
    {
        let (authority, path) = after_slashes.split_once('/').ok_or(TargetError::Invalid)?;
        if authority.eq_ignore_ascii_case("localhost") {
            format!("/{}", decode_percent(path).map_err(TargetError::Decode)?)
        } else if cfg!(windows)
            && !authority.is_empty()
            && authority
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
            && !path.is_empty()
        {
            let path = decode_percent(path).map_err(TargetError::Decode)?;
            format!("\\\\{authority}\\{}", path.replace('/', "\\"))
        } else {
            return Err(TargetError::Unsupported);
        }
    } else {
        let path = rest.strip_prefix("//").unwrap_or(rest);
        decode_percent(path).map_err(TargetError::Decode)?
    };
    let path = if cfg!(windows) && path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
        path[1..].to_owned()
    } else {
        path
    };
    if !std::path::Path::new(&path).is_absolute() {
        return Err(TargetError::Invalid);
    }
    local_parts(path, anchor)
}

#[derive(Debug)]
pub enum DecodeError {
    Escape,
    Control,
    Utf8(FromUtf8Error),
}

pub fn decode_percent(value: &str) -> Result<String, DecodeError> {
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex = |byte: u8| char::from(byte).to_digit(16).map(|x| x as u8);
            let hi = bytes.next().and_then(hex).ok_or(DecodeError::Escape)?;
            let lo = bytes.next().and_then(hex).ok_or(DecodeError::Escape)?;
            out.push(hi * 16 + lo);
        } else {
            out.push(byte);
        }
    }
    if out.iter().any(|byte| byte.is_ascii_control()) {
        return Err(DecodeError::Control);
    }
    String::from_utf8(out).map_err(DecodeError::Utf8)
}
