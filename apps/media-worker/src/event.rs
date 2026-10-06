//! S3 event notifications delivered through SQS.

use serde::Deserialize;

/// One uploaded object named by an S3 `ObjectCreated` notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub bucket: String,
    /// Decoded object key (`incoming/{assetId}`).
    pub key: String,
    pub asset_id: String,
    pub size: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Notification {
    /// `s3:TestEvent`, sent once when the notification is configured.
    Test,
    Uploads {
        uploads: Vec<Upload>,
        /// Records that are not `incoming/{assetId}` objects (logged, ignored).
        ignored: usize,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("message body is not an S3 event notification")]
pub struct NotAnS3Event;

#[derive(Deserialize)]
struct Body {
    #[serde(rename = "Event")]
    event: Option<String>,
    #[serde(rename = "Records")]
    records: Option<Vec<Record>>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "eventName")]
    event_name: Option<String>,
    s3: Option<S3Entity>,
}

#[derive(Deserialize)]
struct S3Entity {
    bucket: Option<Bucket>,
    object: Option<Object>,
}

#[derive(Deserialize)]
struct Bucket {
    name: Option<String>,
}

#[derive(Deserialize)]
struct Object {
    key: Option<String>,
    size: Option<i64>,
}

/// Parses an SQS message body holding an S3 event notification.
pub fn parse(body: &str) -> Result<Notification, NotAnS3Event> {
    let body: Body = serde_json::from_str(body).map_err(|_| NotAnS3Event)?;
    if body.event.as_deref() == Some("s3:TestEvent") {
        return Ok(Notification::Test);
    }
    let records = body.records.ok_or(NotAnS3Event)?;
    let mut uploads = Vec::new();
    let mut ignored = 0;
    for record in records {
        let created = record
            .event_name
            .as_deref()
            .is_none_or(|name| name.starts_with("ObjectCreated:"));
        let parsed = record.s3.filter(|_| created).and_then(|s3| {
            let bucket = s3.bucket?.name?;
            let object = s3.object?;
            let key = decode_key(&object.key?)?;
            let asset_id = asset_id(&key)?.to_owned();
            Some(Upload {
                bucket,
                key,
                asset_id,
                size: object.size,
            })
        });
        match parsed {
            Some(upload) => uploads.push(upload),
            None => ignored += 1,
        }
    }
    Ok(Notification::Uploads { uploads, ignored })
}

/// S3 notification keys are URL-encoded with `+` for spaces.
pub fn decode_key(key: &str) -> Option<String> {
    let bytes = key.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let hex = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `incoming/{assetId}` where the id is 1–64 ASCII alphanumerics.
pub fn asset_id(key: &str) -> Option<&str> {
    let id = key.strip_prefix("incoming/")?;
    (!id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, size: i64) -> String {
        format!(
            r#"{{"eventVersion":"2.1","eventSource":"aws:s3","awsRegion":"us-east-1","eventName":"ObjectCreated:Put","s3":{{"s3SchemaVersion":"1.0","bucket":{{"name":"caper-incoming","arn":"arn:aws:s3:::caper-incoming"}},"object":{{"key":"{key}","size":{size},"eTag":"abc","sequencer":"00"}}}}}}"#
        )
    }

    #[test]
    fn parses_object_created_records() {
        let body = format!(r#"{{"Records":[{}]}}"#, record("incoming/AbC123", 42));
        assert_eq!(
            parse(&body).unwrap(),
            Notification::Uploads {
                uploads: vec![Upload {
                    bucket: "caper-incoming".into(),
                    key: "incoming/AbC123".into(),
                    asset_id: "AbC123".into(),
                    size: Some(42),
                }],
                ignored: 0
            }
        );
    }

    #[test]
    fn decodes_url_encoded_keys_and_ignores_foreign_keys() {
        let body = format!(
            r#"{{"Records":[{},{},{},{}]}}"#,
            record("incoming%2FAbC", 1),
            record("incoming/has+space", 1),
            record("other/AbC", 1),
            record("incoming/a%2Fb", 1)
        );
        let Notification::Uploads { uploads, ignored } = parse(&body).unwrap() else {
            panic!("expected uploads");
        };
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].asset_id, "AbC");
        assert_eq!(ignored, 3);
    }

    #[test]
    fn recognises_the_test_event() {
        let body = r#"{"Service":"Amazon S3","Event":"s3:TestEvent","Time":"2026-10-06T00:00:00.000Z","Bucket":"caper-incoming","RequestId":"X","HostId":"Y"}"#;
        assert_eq!(parse(body).unwrap(), Notification::Test);
    }

    #[test]
    fn rejects_non_s3_bodies() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"hello":1}"#).is_err());
    }

    #[test]
    fn ignores_non_create_events() {
        let body = format!(r#"{{"Records":[{}]}}"#, record("incoming/AbC", 1))
            .replace("ObjectCreated:Put", "ObjectRemoved:Delete");
        assert_eq!(
            parse(&body).unwrap(),
            Notification::Uploads {
                uploads: vec![],
                ignored: 1
            }
        );
    }

    #[test]
    fn key_rules() {
        assert_eq!(decode_key("a%20b+c").as_deref(), Some("a b c"));
        assert_eq!(decode_key("bad%2"), None);
        assert_eq!(decode_key("bad%zz"), None);
        assert_eq!(asset_id("incoming/abcXYZ019"), Some("abcXYZ019"));
        assert_eq!(
            asset_id(&format!("incoming/{}", "a".repeat(64))).map(str::len),
            Some(64)
        );
        assert_eq!(asset_id(&format!("incoming/{}", "a".repeat(65))), None);
        assert_eq!(asset_id("incoming/"), None);
        assert_eq!(asset_id("incoming/a-b"), None);
        assert_eq!(asset_id("incoming/a/b"), None);
        assert_eq!(asset_id("incoming/é"), None);
        assert_eq!(asset_id("original/abc"), None);
    }
}
