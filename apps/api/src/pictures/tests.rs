use super::*;

fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes.extend([0xf7, 0, 0]);
    bytes
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes
}

fn webp(chunk: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
    bytes.extend(chunk);
    bytes.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
    bytes.extend(data);
    bytes
}

/// A JPEG whose frame header sits behind a large APP1 (EXIF) segment.
fn jpeg(width: u16, height: u16, exif: usize) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
    bytes.extend(u16::try_from(exif + 2).unwrap().to_be_bytes());
    bytes.resize(bytes.len() + exif, 0);
    // Fill bytes before a marker are legal.
    bytes.extend([0xff, 0xff, 0xc4, 0x00, 0x04, 0, 0]);
    bytes.extend([0xff, 0xc2, 0x00, 0x11, 8]);
    bytes.extend(height.to_be_bytes());
    bytes.extend(width.to_be_bytes());
    bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    bytes
}

#[test]
fn dimensions_come_from_the_stored_bytes() {
    assert_eq!(dimensions("image/gif", &gif(480, 270)), Some((480, 270)));
    assert_eq!(dimensions("image/png", &png(512, 512)), Some((512, 512)));
    assert_eq!(
        dimensions("image/jpeg", &jpeg(640, 480, 20_000)),
        Some((640, 480))
    );
    // Lossy, lossless and extended WebP.
    let mut lossy = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
    lossy.extend(300u16.to_le_bytes());
    lossy.extend((200u16 | 0x4000).to_le_bytes());
    assert_eq!(
        dimensions("image/webp", &webp(b"VP8 ", &lossy)),
        Some((300, 200))
    );
    let bits: u32 = (512 - 1) | ((256 - 1) << 14);
    let mut lossless = vec![0x2f];
    lossless.extend(bits.to_le_bytes());
    assert_eq!(
        dimensions("image/webp", &webp(b"VP8L", &lossless)),
        Some((512, 256))
    );
    let mut extended = vec![0x10, 0, 0, 0];
    extended.extend(&(1023u32).to_le_bytes()[..3]);
    extended.extend(&(99u32).to_le_bytes()[..3]);
    assert_eq!(
        dimensions("image/webp", &webp(b"VP8X", &extended)),
        Some((1024, 100))
    );
}

#[test]
fn unreadable_or_mislabelled_pictures_have_no_dimensions() {
    assert_eq!(dimensions("image/png", &gif(10, 10)), None);
    assert_eq!(dimensions("image/gif", b"GIF89a\x01"), None);
    assert_eq!(
        dimensions("image/jpeg", &[0xff, 0xd8, 0xff, 0xda, 0, 2]),
        None
    );
    // A frame header beyond the bytes read cannot be trusted.
    let far = jpeg(64, 64, 60_000);
    assert_eq!(dimensions("image/jpeg", &far[..1024]), None);
    assert_eq!(dimensions("image/svg+xml", b"<svg"), None);
    assert_eq!(dimensions("image/webp", &webp(b"ALPH", &[0; 16])), None);
}

#[test]
fn stored_pictures_must_fit_1024_pixels() {
    assert!(fits("image/gif", &gif(1024, 1024)));
    assert!(!fits("image/gif", &gif(1025, 10)));
    assert!(!fits("image/gif", &gif(0, 10)));
    assert!(!fits("image/png", &png(10, 4000)));
}

#[test]
fn reservations_check_type_and_dimensions_but_not_bytes() {
    assert!(check_reservation("image/webp", Some(512), Some(512)).is_ok());
    assert!(check_reservation("image/gif", Some(480), Some(480)).is_ok());
    let code = |result: Result<(), ApiError>| result.unwrap_err().code;
    assert_eq!(
        code(check_reservation("image/avif", Some(1), Some(1))),
        Some("picture_type")
    );
    assert_eq!(
        code(check_reservation("image/svg+xml", Some(1), Some(1))),
        Some("picture_type")
    );
    for (width, height) in [(None, Some(1)), (Some(1025), Some(1)), (Some(0), Some(1))] {
        assert_eq!(
            check_reservation("image/png", width, height)
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
    }
}

#[test]
fn purposes_parse_from_the_wire_and_the_database() {
    assert_eq!(Purpose::parse(None).unwrap(), Purpose::Attachment);
    assert_eq!(Purpose::parse(Some("avatar")).unwrap(), Purpose::Avatar);
    assert_eq!(
        Purpose::parse(Some("spaceIcon")).unwrap(),
        Purpose::SpaceIcon
    );
    assert!(Purpose::parse(Some("space_icon")).is_err());
    for purpose in [Purpose::Attachment, Purpose::Avatar, Purpose::SpaceIcon] {
        assert_eq!(Purpose::from_column(purpose.column()), purpose);
    }
}

#[test]
fn signing_adds_urls_beside_picture_ids_everywhere_and_never_for_attachment_ids() {
    let signer = CdnSigner::new("https://cdn.test".into(), vec![7; 32]);
    let picture = "a".repeat(ID_LENGTH);
    let attachment = "b".repeat(16);
    let mut value = json!({
        "account": {"id": "me", "avatarId": 3, "avatarAsset": picture},
        "members": [
            {"id": "x", "avatarId": 1, "avatarAsset": attachment, "avatarUrl": "https://stale"},
            {"id": "y", "avatarId": 2, "avatarAsset": null},
        ],
        "space": {"id": "s", "iconAsset": picture},
        "other": {"iconUrl": "https://stale"},
    });
    sign(&mut value, &signer, 1_700_000_000);
    assert_eq!(
        value["account"]["avatarUrl"],
        json!(signer.url(&original_key(&picture), 1_700_000_000))
    );
    assert_eq!(
        value["space"]["iconUrl"],
        json!(signer.url(&original_key(&picture), 1_700_000_000))
    );
    assert!(value["members"][0].get("avatarUrl").is_none());
    assert!(value["members"][1].get("avatarAsset").is_none());
    assert!(value["other"].get("iconUrl").is_none());
    assert!(
        value["account"]["avatarUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://cdn.test/assets/")
    );
}

#[test]
fn only_responses_mentioning_pictures_are_parsed() {
    assert!(mentioned(br#"{"avatarAsset":"x"}"#));
    assert!(mentioned(br#"[{"iconAsset":null}]"#));
    assert!(!mentioned(br#"{"avatarId":4,"text":"my avatarAsset"}"#));
    // A prefilter: quoted look-alikes in text are parsed, then left alone.
    assert!(mentioned(br#"{"text":""avatarAsset""}"#));
}

#[test]
fn people_are_found_by_account_id_wherever_they_are_nested() {
    let message = json!({
        "author": {"id": "u1", "name": "A", "avatarId": 1, "avatarAsset": "stale"},
        "thread": {"participants": [{"id": "u2", "avatarId": 2}]},
        "replyTo": {"author": {"id": "u3", "avatarId": null}},
        "forward": {"message": {"author": {"id": "u1", "avatarId": 1}}},
        "content": {"attachments": [{"id": "file", "kind": "image"}]},
    });
    let mut found = Vec::new();
    people(&message, &mut |id| found.push(id.to_owned()));
    found.sort();
    assert_eq!(found, ["u1", "u1", "u2", "u3"]);

    let mut message = message;
    let pictures = std::collections::HashMap::from([("u2".to_owned(), "p2".to_owned())]);
    set_pictures(&mut message, &pictures);
    assert!(message["author"].get("avatarAsset").is_none());
    assert_eq!(message["thread"]["participants"][0]["avatarAsset"], "p2");
    assert!(message["replyTo"]["author"].get("avatarAsset").is_none());
    assert!(
        message["content"]["attachments"][0]
            .get("avatarAsset")
            .is_none()
    );
}
