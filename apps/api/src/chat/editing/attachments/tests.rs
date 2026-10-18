use super::*;
use serde_json::json;

fn cases() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../../shared/messages/edit-cases.json"
    ))
    .expect("valid shared edit cases")
}

fn checked(edit: &Value) -> Result<(), Refusal> {
    serde_json::from_value::<EditDescription>(edit.clone())
        .map_err(|_| Refusal::InvalidEdit)?
        .check()
}

fn untouched() -> Value {
    json!({"rotation":0,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":[]})
}

#[test]
fn shared_wire_limits_match_the_api() {
    let limits = &cases()["wire"]["limits"];
    assert_eq!(limits["marks"], MAX_MARKS);
    assert_eq!(limits["pointValues"], MAX_POINT_VALUES);
    assert_eq!(limits["textCharacters"], MAX_TEXT_CHARACTERS);
    assert_eq!(limits["serializedBytes"], MAX_EDIT_BYTES);
    assert_eq!(limits["cropTolerance"], CROP_TOLERANCE);
    assert_eq!(limits["roundTo"], 1e-6);
}

#[test]
fn shared_wire_cases_are_accepted_and_refused_like_every_client() {
    let wire = &cases()["wire"];
    let valid = wire["valid"].as_array().unwrap();
    assert!(valid.len() >= 6, "edit-cases.json lost its valid cases");
    for case in valid {
        assert_eq!(checked(&case["edit"]), Ok(()), "{}", case["note"]);
    }
    let invalid = wire["invalid"].as_array().unwrap();
    assert!(
        invalid.len() >= 20,
        "edit-cases.json lost its invalid cases"
    );
    for case in invalid {
        assert!(checked(&case["edit"]).is_err(), "{}", case["note"]);
    }
    let generated = wire["generated"].as_array().unwrap();
    assert!(generated.len() >= 6);
    for case in generated {
        let mut edit = untouched();
        let marks = edit["marks"].as_array_mut().unwrap();
        if let Some(text) = case["text"].as_str() {
            let repeat = case["repeat"].as_u64().unwrap() as usize;
            marks.push(json!({"kind":"text","color":"#FFFFFF","size":0.05,"x":0.5,"y":0.5,"text":text.repeat(repeat)}));
        } else {
            let points = vec![0.5; case["pointValues"].as_u64().unwrap() as usize];
            for _ in 0..case["strokes"].as_u64().unwrap() {
                marks.push(json!({"kind":"stroke","color":"#000000","width":0.01,"points":points}));
            }
        }
        assert_eq!(
            checked(&edit).is_ok(),
            case["valid"] == true,
            "{}",
            case["note"]
        );
    }
}

/// The API stores what editors send; this keeps the shared rounding results
/// consistent with the rule (and valid) for every client that applies it.
#[test]
fn shared_rounding_cases_follow_the_millionth_rule() {
    fn round(before: &Value, after: &Value, key: &str) {
        match (before, after) {
            (Value::Number(b), Value::Number(a)) => {
                let b = b.as_f64().unwrap();
                let a = a.as_f64().unwrap();
                let expected = if key == "rotation" {
                    b
                } else {
                    (b * 1e6).round() / 1e6
                };
                assert!((expected - a).abs() < 1e-9, "{key}: {b} → {a}");
            }
            (Value::Array(b), Value::Array(a)) => {
                assert_eq!(b.len(), a.len());
                for (b, a) in b.iter().zip(a) {
                    round(b, a, key);
                }
            }
            (Value::Object(b), Value::Object(a)) => {
                assert_eq!(b.keys().collect::<Vec<_>>(), a.keys().collect::<Vec<_>>());
                for (name, b) in b {
                    round(b, &a[name], name);
                }
            }
            (b, a) => assert_eq!(b, a, "{key}"),
        }
    }
    let rounds = cases()["wire"]["round"].as_array().unwrap().clone();
    assert!(rounds.len() >= 2);
    for case in rounds {
        round(&case["before"], &case["after"], "");
        assert_eq!(checked(&case["after"]), Ok(()), "{}", case["note"]);
    }
}

#[test]
fn edit_descriptions_store_exactly_their_fields() {
    let edit: EditDescription = serde_json::from_value(json!({
        "rotation": 90,
        "crop": {"x":0.1,"y":0,"width":0.5,"height":1},
        "marks": [
            {"kind":"stroke","color":"#e5484d","width":0.01,"points":[0.5,0.5]},
            {"kind":"text","color":"#FFFFFF","size":0.05,"x":0.5,"y":0.5,"text":"Hi"},
        ],
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&edit).unwrap(),
        json!({"rotation":90,"crop":{"x":0.1,"y":0.0,"width":0.5,"height":1.0},"marks":[
            {"kind":"stroke","color":"#e5484d","width":0.01,"points":[0.5,0.5]},
            {"kind":"text","color":"#FFFFFF","size":0.05,"x":0.5,"y":0.5,"text":"Hi"}]})
    );
    // Non-finite numbers cannot arrive as JSON, but are refused anyway.
    let mut infinite = edit.clone();
    infinite.crop.width = f64::INFINITY;
    assert_eq!(infinite.check(), Err(Refusal::InvalidEdit));
    let mut tab = edit;
    tab.marks[1] = Mark::Text {
        color: "#FFFFFF".into(),
        size: 0.05,
        x: 0.5,
        y: 0.5,
        text: "a\tb".into(),
    };
    assert_eq!(tab.check(), Err(Refusal::InvalidEdit));
    let too_large = ApiError::from(Refusal::EditTooLarge);
    assert_eq!(
        (too_large.status, too_large.code),
        (StatusCode::BAD_REQUEST, Some("edit_too_large"))
    );
}

fn change(
    id: &str,
    spoiler: bool,
    replaces: Option<&str>,
    edit: Option<Value>,
) -> AttachmentChange {
    serde_json::from_value(json!({"id":id,"spoiler":spoiler,"replaces":replaces,"edit":edit}))
        .unwrap()
}

fn file(id: &str, kind: &str, source: Option<&str>) -> CurrentFile {
    CurrentFile {
        id: id.into(),
        kind: kind.into(),
        spoiler: false,
        source: source.map(str::to_owned),
        available: true,
    }
}

#[test]
fn requests_name_each_file_once_and_edit_only_replacements() {
    let edit = Some(untouched());
    assert_eq!(check_request(&[]), Ok(()));
    assert_eq!(
        check_request(&[
            change("A", true, None, None),
            change("N", false, Some("B"), edit.clone())
        ]),
        Ok(())
    );
    let eleven: Vec<_> = (0..11)
        .map(|i| change(&format!("F{i}"), false, None, None))
        .collect();
    assert_eq!(check_request(&eleven), Err(Refusal::TooMany));
    assert_eq!(
        check_request(&[
            change("A", false, None, None),
            change("A", true, None, None)
        ]),
        Err(Refusal::TooMany)
    );
    assert_eq!(
        check_request(&[
            change("N1", false, Some("B"), edit.clone()),
            change("N2", false, Some("B"), edit.clone())
        ]),
        Err(Refusal::TooMany)
    );
    assert_eq!(
        check_request(&[change("A", false, Some("A"), None)]),
        Err(Refusal::Mismatch)
    );
    assert_eq!(
        check_request(&[change("A", false, None, edit)]),
        Err(Refusal::EditWithoutReplacement)
    );
    assert_eq!(
        check_request(&[change(
            "N",
            false,
            Some("B"),
            Some(json!({"rotation":45,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":[]}))
        )]),
        Err(Refusal::InvalidEdit)
    );
    // Unknown fields in an entry are refused while the body is read (422).
    assert!(serde_json::from_value::<AttachmentChange>(json!({"id":"A","remove":true})).is_err());
    let defaults: AttachmentChange = serde_json::from_value(json!({"id":"A"})).unwrap();
    assert!(!defaults.spoiler && defaults.replaces.is_none() && defaults.edit.is_none());
}

#[test]
fn plans_keep_replace_restore_and_toggle_spoilers_slot_by_slot() {
    let edit = untouched();
    let current = [
        file("A", "file", None),
        file("B", "image", None),
        file("N0", "video", Some("V")),
    ];
    let plan = plan(
        &current,
        &[
            change("A", true, None, None),
            change("N1", false, Some("B"), Some(edit.clone())),
            change("V", true, Some("N0"), None),
        ],
    )
    .unwrap();
    let description: EditDescription = serde_json::from_value(edit.clone()).unwrap();
    assert_eq!(
        plan.files,
        vec![
            PlannedFile {
                id: "A".into(),
                spoiler: true,
                change: Change::Keep
            },
            PlannedFile {
                id: "N1".into(),
                spoiler: false,
                change: Change::Upload {
                    retire: "B".into(),
                    source: "B".into(),
                    kind: "image".into(),
                    edit: description.clone()
                }
            },
            PlannedFile {
                id: "V".into(),
                spoiler: true,
                change: Change::Restore {
                    retire: "N0".into()
                }
            },
        ]
    );
    assert_eq!(plan.uploads(), ["N1"]);
    assert_eq!(plan.restores(), ["V"]);
    assert_eq!(plan.retired(), ["B", "N0"]);
    assert!(plan.changes(&current));

    // Re-editing a render starts from the slot's first-sent file again.
    let chained = super::plan(
        &[file("N1", "image", Some("B"))],
        &[change("N2", false, Some("N1"), Some(edit.clone()))],
    )
    .unwrap();
    assert_eq!(
        chained.files[0].change,
        Change::Upload {
            retire: "N1".into(),
            source: "B".into(),
            kind: "image".into(),
            edit: description
        }
    );

    // A spoiler toggle alone changes the files; the same list does not.
    let same = super::plan(&current[..1], &[change("A", false, None, None)]).unwrap();
    assert!(!same.changes(&current[..1]));
    let toggled = super::plan(&current[..1], &[change("A", true, None, None)]).unwrap();
    assert!(toggled.changes(&current[..1]));
    assert!(toggled.retired().is_empty() && toggled.uploads().is_empty());
}

#[test]
fn plans_refuse_lists_that_do_not_match_the_message() {
    let edit = Some(untouched());
    let current = [file("A", "image", None), file("B", "image", None)];
    for (changes, refusal) in [
        // Files cannot be removed or added after sending.
        (vec![change("A", false, None, None)], Refusal::Mismatch),
        (
            vec![
                change("A", false, None, None),
                change("B", false, None, None),
                change("C", false, None, None),
            ],
            Refusal::Mismatch,
        ),
        // Reordered, or naming a file the message does not show.
        (
            vec![
                change("B", false, None, None),
                change("A", false, None, None),
            ],
            Refusal::Mismatch,
        ),
        (
            vec![
                change("A", false, None, None),
                change("N", false, Some("A"), edit.clone()),
            ],
            Refusal::Mismatch,
        ),
        // A replacement cannot be another file the message shows.
        (
            vec![
                change("A", false, None, None),
                change("A", false, Some("B"), None),
            ],
            Refusal::Mismatch,
        ),
        // A new upload carries its edit; a first-sent file never does.
        (
            vec![
                change("A", false, None, None),
                change("N", false, Some("B"), None),
            ],
            Refusal::MissingEdit,
        ),
    ] {
        assert_eq!(plan(&current, &changes), Err(refusal), "{changes:?}");
    }
    assert_eq!(
        plan(
            &[file("N", "image", Some("B"))],
            &[change("B", false, Some("N"), edit.clone())]
        ),
        Err(Refusal::RestoreWithEdit)
    );
    assert_eq!(
        plan(
            &[file("D", "file", None)],
            &[change("N", false, Some("D"), edit.clone())]
        ),
        Err(Refusal::NotEditable)
    );
    assert_eq!(
        plan(
            &[file("S", "audio", None)],
            &[change("N", false, Some("S"), edit.clone())]
        ),
        Err(Refusal::NotEditable)
    );
    let mut gone = file("B", "image", None);
    gone.available = false;
    assert_eq!(
        plan(&[gone], &[change("N", false, Some("B"), edit)]),
        Err(Refusal::Unavailable)
    );
    assert_eq!(
        ApiError::from(Refusal::Unavailable).status,
        StatusCode::CONFLICT
    );
}

#[test]
fn shown_and_requested_lists_compare_ids_and_spoilers() {
    let content = json!({"attachments":[{"id":"A","spoiler":true},{"id":"B"}]});
    assert_eq!(
        shown(content.get("attachments")),
        [("A", true), ("B", false)]
    );
    assert!(shown(None).is_empty());
    assert_eq!(
        requested(&[
            change("A", true, None, None),
            change("B", false, None, None)
        ]),
        shown(content.get("attachments"))
    );
}
