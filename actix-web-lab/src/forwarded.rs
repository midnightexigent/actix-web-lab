//! Forwarded typed header.
//!
//! See [`Forwarded`] docs.

use std::str;

use actix_web::{
    error::ParseError,
    http::header::{self, Header, HeaderName, HeaderValue, TryIntoHeaderValue},
    HttpMessage,
};

/// `Forwarded` header, defined in [RFC 7239].
///
/// [RFC 7239]: https://datatracker.ietf.org/doc/html/rfc7239
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(Default))]
pub struct Forwarded {
    by: Option<String>,
    r#for: Vec<String>,
    host: Option<String>,
    proto: Option<String>,
}

impl Forwarded {
    /// Returns first `for` parameter which is typically the client's identifier.
    pub fn for_client(&self) -> Option<&str> {
        // Taking the first value for each property is correct because spec states that first "for"
        // value is client and rest are proxies. We collect them in the order they are read.
        //
        // ```plain
        // > In a chain of proxy servers where this is fully utilized, the first
        // > "for" parameter will disclose the client where the request was first
        // > made, followed by any subsequent proxy identifiers.
        // - https://datatracker.ietf.org/doc/html/rfc7239#section-5.2
        // ```

        self.r#for.first().map(String::as_str)
    }

    fn has_no_info(&self) -> bool {
        self.by.is_none() && self.r#for.is_empty() && self.host.is_none() && self.proto.is_none()
    }

    // TODO: parse with trusted IP ranges fn
}

impl str::FromStr for Forwarded {
    type Err = <usize as str::FromStr>::Err;

    #[inline]
    fn from_str(val: &str) -> Result<Self, Self::Err> {
        let mut host = None;
        let mut proto = None;
        let mut r#for = vec![];

        // "for=1.2.3.4, for=5.6.7.8; scheme=https"
        for (name, val) in val
            .split(';')
            // ["for=1.2.3.4, for=5.6.7.8", " proto=https"]
            .flat_map(|vals| vals.split(','))
            // ["for=1.2.3.4", " for=5.6.7.8", " proto=https"]
            .flat_map(|pair| {
                let mut items = pair.trim().splitn(2, '=');
                Some((items.next()?, items.next()?))
            })
        {
            // [(name , val      ), ...                                    ]
            // [("for", "1.2.3.4"), ("for", "5.6.7.8"), ("scheme", "https")]

            match name.trim().to_lowercase().as_str() {
                "by" => {
                    // TODO: implement https://datatracker.ietf.org/doc/html/rfc7239#section-5.1
                    continue;
                }
                "for" => {
                    // parameter order is defined to be client first and last proxy last
                    r#for.push(unquote(val));
                }
                "host" => {
                    // multiple values on other properties have no defined semantics
                    host.get_or_insert_with(|| unquote(val));
                }
                "proto" => {
                    // multiple values on other properties have no defined semantics
                    proto.get_or_insert_with(|| unquote(val));
                }
                _ => continue,
            };
        }

        Ok(Self {
            by: None,
            r#for: r#for.into_iter().map(str::to_owned).collect(),
            host: host.map(str::to_owned),
            proto: proto.map(str::to_owned),
        })
    }
}

impl TryIntoHeaderValue for Forwarded {
    type Error = header::InvalidHeaderValue;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        if self.has_no_info() {
            return Ok(HeaderValue::from_static(""));
        }

        let r#for = if self.r#for.is_empty() {
            None
        } else {
            let value = self
                .r#for
                .into_iter()
                .map(|ident| format!("for=\"{ident}\""))
                .collect::<Vec<_>>()
                .join(", ");

            Some(value)
        };

        // it has been chosen to quote all values to avoid overhead of detecting whether quotes are
        // needed or not in the case values containing IPv6 addresses, for example

        self.by
            .map(|by| format!("by=\"{by}\""))
            .into_iter()
            .chain(r#for)
            .chain(self.host.map(|host| format!("host=\"{host}\"")))
            .chain(self.proto.map(|proto| format!("proto=\"{proto}\"")))
            .collect::<Vec<_>>()
            .join("; ")
            .try_into_value()
    }
}

impl Header for Forwarded {
    fn name() -> HeaderName {
        header::FORWARDED
    }

    fn parse<M: HttpMessage>(msg: &M) -> Result<Self, ParseError> {
        let combined = msg
            .headers()
            .get_all(Self::name())
            .filter_map(|hdr| hdr.to_str().ok())
            .filter_map(|hdr_str| match hdr_str.trim() {
                "" => None,
                val => Some(val),
            })
            .collect::<Vec<_>>();

        if combined.is_empty() {
            return Err(ParseError::Header);
        }

        // pass to FromStr impl as if it were one concatenated header with semicolon joiners
        // https://datatracker.ietf.org/doc/html/rfc7239#section-7.1
        combined.join(";").parse().map_err(|_| ParseError::Header)
    }
}

/// Trim whitespace then any quote marks.
fn unquote(val: &str) -> &str {
    val.trim().trim_start_matches('"').trim_end_matches('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{assert_parse_eq, assert_parse_fail};

    #[test]
    fn missing_header() {
        assert_parse_fail::<Forwarded, _, _>([""; 0]);
        assert_parse_fail::<Forwarded, _, _>([""]);
    }

    #[test]
    fn parsing_header_parts() {
        assert_parse_eq::<Forwarded, _, _>([";"], Forwarded::default());

        assert_parse_eq::<Forwarded, _, _>(
            ["for=192.0.2.60; proto=https; by=203.0.113.43; host=rust-lang.org"],
            Forwarded {
                host: Some("rust-lang.org".to_owned()),
                proto: Some("https".to_owned()),
                r#for: vec!["192.0.2.60".to_owned()],
                // by: Some("203.0.113.43".to_owned()),
                by: None,
            },
        );

        assert_parse_eq::<Forwarded, _, _>(
            ["for=192.0.2.60; proto=https", "host=rust-lang.org"],
            Forwarded {
                by: None,
                host: Some("rust-lang.org".to_owned()),
                r#for: vec!["192.0.2.60".to_owned()],
                proto: Some("https".to_owned()),
            },
        );
    }

    #[test]
    fn serializing() {
        let fwd = Forwarded {
            by: Some("203.0.113.43".to_owned()),
            r#for: vec!["192.0.2.60".to_owned()],
            host: Some("rust-lang.org".to_owned()),
            proto: Some("https".to_owned()),
        };

        assert_eq!(
            fwd.try_into_value().unwrap(),
            r#"by="203.0.113.43"; for="192.0.2.60"; host="rust-lang.org"; proto="https""#
        );
    }

    #[test]
    fn case_sensitivity() {
        assert_parse_eq::<Forwarded, _, _>(
            ["For=192.0.2.60"],
            Forwarded {
                r#for: vec!["192.0.2.60".to_owned()],
                ..Forwarded::default()
            },
        );
    }

    #[test]
    fn weird_whitespace() {
        assert_parse_eq::<Forwarded, _, _>(
            ["for= 1.2.3.4; proto= https"],
            Forwarded {
                r#for: vec!["1.2.3.4".to_owned()],
                proto: Some("https".to_owned()),
                ..Forwarded::default()
            },
        );

        assert_parse_eq::<Forwarded, _, _>(
            ["  for = 1.2.3.4  "],
            Forwarded {
                r#for: vec!["1.2.3.4".to_owned()],
                ..Forwarded::default()
            },
        );
    }

    #[test]
    fn for_quoted() {
        assert_parse_eq::<Forwarded, _, _>(
            [r#"for="192.0.2.60:8080""#],
            Forwarded {
                r#for: vec!["192.0.2.60:8080".to_owned()],
                ..Forwarded::default()
            },
        );
    }

    #[test]
    fn for_ipv6() {
        assert_parse_eq::<Forwarded, _, _>(
            [r#"for="[2001:db8:cafe::17]:4711""#],
            Forwarded {
                r#for: vec!["[2001:db8:cafe::17]:4711".to_owned()],
                ..Forwarded::default()
            },
        );
    }

    #[test]
    fn for_multiple() {
        let fwd = Forwarded {
            r#for: vec!["192.0.2.60".to_owned(), "198.51.100.17".to_owned()],
            ..Forwarded::default()
        };

        assert_eq!(fwd.for_client().unwrap(), "192.0.2.60");

        assert_parse_eq::<Forwarded, _, _>(["for=192.0.2.60, for=198.51.100.17"], fwd);
    }
}