//! Multipart uploads for files too large for one `PUT`. Clients upload parts
//! with presigned `PUT`s; R2 only accepts the requests that start, list,
//! finish and copy them with signed headers, so those stay on the server.
use super::{R2, unavailable};
use crate::ApiError;

/// Parts are at least this large (R2's minimum is 5 MiB) so a 10 GB file
/// stays near 160 parts; larger files grow the part size instead.
pub(super) const PART_BYTES: i64 = 64 * 1024 * 1024;
/// R2's limits: parts per upload, and bytes per `PUT` or per copied part.
const MAX_PARTS: i64 = 10_000;
pub(super) const MAX_SINGLE_BYTES: i64 = 5 * 1024 * 1024 * 1024;
/// Server-side copies move this much per part.
const COPY_PART_BYTES: i64 = 1024 * 1024 * 1024;

/// The part size for a file of `bytes`: whole MiB, and few enough parts.
pub(super) fn part_bytes(bytes: i64) -> i64 {
    const MIB: i64 = 1024 * 1024;
    let needed = (bytes + MAX_PARTS - 1) / MAX_PARTS;
    PART_BYTES.max((needed + MIB - 1) / MIB * MIB)
}

pub(super) fn part_count(bytes: i64, part_bytes: i64) -> i64 {
    (bytes + part_bytes - 1) / part_bytes
}

/// The size part `number` (from 1) must have.
fn expected_part(bytes: i64, part_bytes: i64, number: i64) -> i64 {
    if number < part_count(bytes, part_bytes) {
        part_bytes
    } else {
        bytes - part_bytes * (number - 1)
    }
}

/// The text of the first `<name>` element, as stored (still XML-escaped).
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    elements(xml, name).next()
}

fn elements<'a>(xml: &'a str, name: &str) -> impl Iterator<Item = &'a str> + use<'a> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut rest = xml;
    std::iter::from_fn(move || {
        let start = rest.find(&open)? + open.len();
        let end = start + rest[start..].find(&close)?;
        let found = &rest[start..end];
        rest = &rest[end + close.len()..];
        Some(found)
    })
}

/// R2 (like S3) can answer 200 with an `<Error>` body for long operations.
fn failed(body: &str) -> bool {
    body.contains("<Error>")
}

/// One uploaded part as R2 lists it.
#[derive(Debug, PartialEq)]
pub(super) struct Part {
    pub number: i64,
    /// Kept XML-escaped: it goes straight back into the completion request.
    pub etag: String,
    pub size: i64,
}

/// Why the uploaded parts can't be assembled yet.
#[derive(Debug, PartialEq)]
pub(super) enum Incomplete {
    /// Some parts were never uploaded; the client should send them.
    Missing,
    /// A part has the wrong size; the upload can't match its reservation.
    Mismatched,
}

/// Checks the listed parts against the reservation and returns them in order.
pub(super) fn assemble(
    mut parts: Vec<Part>,
    bytes: i64,
    part_bytes: i64,
) -> Result<Vec<Part>, Incomplete> {
    parts.sort_by_key(|part| part.number);
    let count = part_count(bytes, part_bytes);
    if parts
        .iter()
        .any(|part| part.number < 1 || part.number > count)
    {
        return Err(Incomplete::Mismatched);
    }
    if parts
        .iter()
        .any(|part| part.size != expected_part(bytes, part_bytes, part.number))
    {
        return Err(Incomplete::Mismatched);
    }
    if parts.len() as i64 != count {
        return Err(Incomplete::Missing);
    }
    Ok(parts)
}

fn completion(parts: &[Part]) -> String {
    let mut body = String::from("<CompleteMultipartUpload>");
    for part in parts {
        body.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
            part.number, part.etag
        ));
    }
    body.push_str("</CompleteMultipartUpload>");
    body
}

impl R2 {
    fn object(&self, key: &str) -> String {
        format!("/{}/{key}", self.bucket)
    }

    async fn send(
        &self,
        method: reqwest::Method,
        key: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<String, ApiError> {
        let (url, headers) =
            self.signer()
                .authorize(method.as_str(), &self.object(key), query, headers);
        let mut request = self.http.request(method, url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        // Copies and starts have no body but still need `Content-Length: 0`.
        request = request.body(body.unwrap_or_default());
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let body = response.text().await.map_err(|_| unavailable())?;
        if failed(&body) {
            return Err(unavailable());
        }
        Ok(body)
    }

    /// Starts a multipart upload to `key` and returns its upload ID.
    pub(super) async fn start_multipart(
        &self,
        key: &str,
        headers: &[(&str, &str)],
    ) -> Result<String, ApiError> {
        let body = self
            .send(
                reqwest::Method::POST,
                key,
                &[("uploads", "")],
                headers,
                None,
            )
            .await?;
        element(&body, "UploadId")
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(unavailable)
    }

    /// A presigned `PUT` for one part. Parts carry no signed headers, so
    /// browsers send them as plain bodies.
    pub(super) fn part_url(&self, key: &str, upload_id: &str, number: i64, expires: u64) -> String {
        let number = number.to_string();
        self.signer().presign(
            "PUT",
            &self.object(key),
            expires,
            &[("partNumber", &number), ("uploadId", upload_id)],
            &[],
        )
    }

    /// Every part uploaded so far.
    pub(super) async fn list_parts(
        &self,
        key: &str,
        upload_id: &str,
    ) -> Result<Vec<Part>, ApiError> {
        let mut parts = Vec::new();
        let mut marker = String::from("0");
        loop {
            let body = self
                .send(
                    reqwest::Method::GET,
                    key,
                    &[
                        ("max-parts", "1000"),
                        ("part-number-marker", &marker),
                        ("uploadId", upload_id),
                    ],
                    &[],
                    None,
                )
                .await?;
            for part in elements(&body, "Part") {
                let number = element(part, "PartNumber").and_then(|v| v.parse().ok());
                let size = element(part, "Size").and_then(|v| v.parse().ok());
                let etag = element(part, "ETag");
                let (Some(number), Some(size), Some(etag)) = (number, size, etag) else {
                    return Err(unavailable());
                };
                parts.push(Part {
                    number,
                    etag: etag.to_owned(),
                    size,
                });
            }
            match (
                element(&body, "IsTruncated"),
                element(&body, "NextPartNumberMarker"),
            ) {
                (Some("true"), Some(next)) if next != marker => marker = next.to_owned(),
                _ => return Ok(parts),
            }
        }
    }

    /// Assembles the listed parts into the object.
    pub(super) async fn complete_multipart(
        &self,
        key: &str,
        upload_id: &str,
        parts: &[Part],
    ) -> Result<(), ApiError> {
        self.send(
            reqwest::Method::POST,
            key,
            &[("uploadId", upload_id)],
            &[("content-type", "application/xml")],
            Some(completion(parts)),
        )
        .await
        .map(drop)
    }

    /// Discards an unfinished upload and its parts, so its part URLs stop
    /// working. One already finished or aborted counts as done.
    pub(super) async fn abort_multipart(&self, key: &str, upload_id: &str) -> Result<(), ()> {
        let (url, headers) =
            self.signer()
                .authorize("DELETE", &self.object(key), &[("uploadId", upload_id)], &[]);
        let mut request = self.http.delete(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(|_| ())?;
        (response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND)
            .then_some(())
            .ok_or(())
    }

    /// Copies `source` to `key` inside the bucket, with new headers, without
    /// downloading it. Objects over 5 GiB copy in parts.
    pub(super) async fn copy(
        &self,
        source: &str,
        key: &str,
        bytes: i64,
        headers: &[(&str, &str)],
    ) -> Result<(), ApiError> {
        let from = self.object(source);
        if bytes <= MAX_SINGLE_BYTES {
            let mut all = vec![
                ("x-amz-copy-source", from.as_str()),
                ("x-amz-metadata-directive", "REPLACE"),
            ];
            all.extend_from_slice(headers);
            return self
                .send(reqwest::Method::PUT, key, &[], &all, None)
                .await
                .map(drop);
        }
        let upload_id = self.start_multipart(key, headers).await?;
        let copied = self.copy_parts(&from, key, &upload_id, bytes).await;
        let finished = match copied {
            Ok(parts) => self.complete_multipart(key, &upload_id, &parts).await,
            Err(error) => Err(error),
        };
        if finished.is_err() {
            let _ = self.abort_multipart(key, &upload_id).await;
        }
        finished
    }

    async fn copy_parts(
        &self,
        from: &str,
        key: &str,
        upload_id: &str,
        bytes: i64,
    ) -> Result<Vec<Part>, ApiError> {
        use futures_util::{StreamExt, TryStreamExt};
        futures_util::stream::iter(1..=part_count(bytes, COPY_PART_BYTES))
            .map(|number| async move {
                let start = (number - 1) * COPY_PART_BYTES;
                let end = (start + COPY_PART_BYTES).min(bytes) - 1;
                let range = format!("bytes={start}-{end}");
                let part = number.to_string();
                let body = self
                    .send(
                        reqwest::Method::PUT,
                        key,
                        &[("partNumber", &part), ("uploadId", upload_id)],
                        &[
                            ("x-amz-copy-source", from),
                            ("x-amz-copy-source-range", &range),
                        ],
                        None,
                    )
                    .await?;
                let etag = element(&body, "ETag").ok_or_else(unavailable)?;
                Ok(Part {
                    number,
                    etag: etag.to_owned(),
                    size: end - start + 1,
                })
            })
            .buffered(4)
            .try_collect()
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_sizes_cover_the_file_in_at_most_ten_thousand_parts() {
        assert_eq!(part_bytes(1), PART_BYTES);
        assert_eq!(part_bytes(10 * 1024 * 1024 * 1024), PART_BYTES);
        let huge = 2 * 1024 * 1024 * 1024 * 1024_i64;
        let size = part_bytes(huge);
        assert_eq!(size % (1024 * 1024), 0);
        assert!(part_count(huge, size) <= MAX_PARTS);
        assert_eq!(part_count(PART_BYTES * 2 + 1, PART_BYTES), 3);
        assert_eq!(expected_part(PART_BYTES * 2 + 1, PART_BYTES, 3), 1);
        assert_eq!(expected_part(PART_BYTES * 2 + 1, PART_BYTES, 2), PART_BYTES);
    }

    #[test]
    fn parts_assemble_only_when_every_one_has_its_reserved_size() {
        let part = |number, size| Part {
            number,
            etag: format!("&quot;{number}&quot;"),
            size,
        };
        let bytes = PART_BYTES * 2 + 10;
        let ready = assemble(
            vec![part(3, 10), part(1, PART_BYTES), part(2, PART_BYTES)],
            bytes,
            PART_BYTES,
        )
        .unwrap();
        assert_eq!(
            ready.iter().map(|p| p.number).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            completion(&ready[..1]),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;1&quot;</ETag></Part></CompleteMultipartUpload>"
        );
        assert_eq!(
            assemble(vec![part(1, PART_BYTES), part(3, 10)], bytes, PART_BYTES),
            Err(Incomplete::Missing)
        );
        assert_eq!(
            assemble(
                vec![part(1, PART_BYTES), part(2, PART_BYTES), part(3, 11)],
                bytes,
                PART_BYTES
            ),
            Err(Incomplete::Mismatched)
        );
        assert_eq!(
            assemble(vec![part(4, 10)], bytes, PART_BYTES),
            Err(Incomplete::Mismatched)
        );
    }

    #[test]
    fn listings_are_read_from_r2_xml() {
        let body = "<ListPartsResult><IsTruncated>true</IsTruncated><NextPartNumberMarker>2</NextPartNumberMarker>\
            <Part><PartNumber>1</PartNumber><ETag>&quot;a&quot;</ETag><Size>5</Size></Part>\
            <Part><PartNumber>2</PartNumber><ETag>&quot;b&quot;</ETag><Size>6</Size></Part></ListPartsResult>";
        let parts: Vec<_> = elements(body, "Part").collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(element(parts[1], "ETag"), Some("&quot;b&quot;"));
        assert_eq!(element(body, "NextPartNumberMarker"), Some("2"));
        assert!(failed(
            "<?xml version=\"1.0\"?><Error><Code>InternalError</Code></Error>"
        ));
        assert!(!failed(body));
    }
}

/// The same calls against a real S3 implementation's XML, for example
/// `uv tool run --from 'moto[server]' moto_server -p 5055` with a `test`
/// bucket: `S3_TEST_ENDPOINT=http://127.0.0.1:5055 cargo test -- --ignored`.
#[cfg(test)]
mod s3 {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an S3-compatible S3_TEST_ENDPOINT on loopback"]
    async fn multipart_upload_list_complete_copy_and_abort_round_trip() {
        // CI runs every ignored test; this one needs its own server.
        let Ok(endpoint) = std::env::var("S3_TEST_ENDPOINT") else {
            return;
        };
        let r2 = R2::new(endpoint, "test".into(), "key".into(), "secret".into());
        let part = vec![b'a'; PART_BYTES as usize];
        let tail = b"tail".to_vec();
        let id = r2
            .start_multipart("staging/original/s3test", &[])
            .await
            .unwrap();
        for (number, bytes) in [(1, &part), (2, &tail)] {
            let url = r2.part_url("staging/original/s3test", &id, number, 60);
            let response = r2.http.put(url).body(bytes.clone()).send().await.unwrap();
            assert!(
                response.status().is_success(),
                "{}",
                response.text().await.unwrap()
            );
        }
        let size = PART_BYTES + 4;
        let listed = r2.list_parts("staging/original/s3test", &id).await.unwrap();
        let parts = assemble(listed, size, PART_BYTES).unwrap();
        r2.complete_multipart("staging/original/s3test", &id, &parts)
            .await
            .unwrap();
        assert_eq!(
            r2.size("staging/original/s3test").await.unwrap(),
            Some(size)
        );
        // One copy in a single request, one through a multipart copy.
        r2.copy(
            "staging/original/s3test",
            "original/s3test",
            size,
            &[("content-type", "application/zip")],
        )
        .await
        .unwrap();
        assert_eq!(r2.size("original/s3test").await.unwrap(), Some(size));
        let copy = r2.start_multipart("original/s3copy", &[]).await.unwrap();
        let parts = r2
            .copy_parts(
                "/test/staging/original/s3test",
                "original/s3copy",
                &copy,
                size,
            )
            .await
            .unwrap();
        r2.complete_multipart("original/s3copy", &copy, &parts)
            .await
            .unwrap();
        assert_eq!(r2.size("original/s3copy").await.unwrap(), Some(size));
        let abandoned = r2
            .start_multipart("staging/original/gone", &[])
            .await
            .unwrap();
        r2.abort_multipart("staging/original/gone", &abandoned)
            .await
            .unwrap();
        assert!(
            r2.list_parts("staging/original/gone", &abandoned)
                .await
                .is_err()
        );
        // Aborting again is harmless.
        r2.abort_multipart("staging/original/gone", &abandoned)
            .await
            .unwrap();
    }
}
