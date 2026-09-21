use super::*;

fn restart_body(generation: Value) -> Value {
    json!({"generation":generation,"sequence":1,"sessionDescription":{"type":"offer",
        "sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=recvonly\r\n"}})
}

async fn request(s: &AppState, route: &str, token: &str, body: Value) -> (StatusCode, Value) {
    call(
        app(s.clone()),
        "POST",
        &format!("/api/media/{route}"),
        Some(token),
        body,
    )
    .await
}

// Run the same contract against memory and real Valkey with independent API instances.
pub(super) async fn exercise_rotation(a: &AppState, b: &AppState, mock: &Mock) {
    let joined = joined(a, "rotating").await;
    let token = joined["token"].as_str().unwrap();
    let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
    let original = joined["turn"]["generation"].clone();
    let session = a
        .read(|r| Ok(r.participants[&id].session.clone()))
        .await
        .unwrap();
    a.update(|r| {
        let cache = r.participants.get_mut(&id).unwrap().turn.as_mut().unwrap();
        cache.issued = Timestamp::now() - Duration::from_secs(TURN_TTL / 2 + 1);
        cache.expires = Timestamp::now() + Duration::from_secs(TURN_TTL / 2 - 1);
        Ok(())
    })
    .await
    .unwrap();
    let (status, renewed) = request(b, "turn", token, json!({"generation":original})).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(renewed["turn"]["generation"], original);
    // Lose that response and retry through a different API, using the old generation.
    let (status, replay) = request(a, "turn", token, json!({"generation":original})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay["iceServers"], renewed["iceServers"]);
    assert_eq!(replay["turn"]["generation"], renewed["turn"]["generation"]);
    assert_eq!(mock.next_turn.load(Ordering::SeqCst), 3);
    let body = restart_body(renewed["turn"]["generation"].clone());
    assert_eq!(
        request(a, "restart-ice", token, body.clone()).await.0,
        StatusCode::OK
    );
    expire_sessions(b).await.unwrap();
    assert_eq!(
        request(b, "snapshot", token, json!({})).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(b, "state", token, json!({"muted":true,"deafened":false}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(b, "restart-ice", token, body).await.0,
        StatusCode::OK
    );
    for s in [a, b] {
        assert_eq!(
            request(
                s,
                "restart-ice-ack",
                token,
                json!({"generation":renewed["turn"]["generation"],"sequence":1})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    b.read(|r| {
        let p = &r.participants[&id];
        assert_eq!(p.session, session);
        assert!(p.muted);
        assert!(!p.operation);
        assert!(p.restart.is_none());
        assert_eq!(
            p.turn_usernames.len(),
            2,
            "retain both still-valid credential generations for leave"
        );
        assert_eq!(p.turn_retired.len(), 1);
        assert!(
            !serde_json::to_string(r).unwrap().contains("a=recvonly"),
            "never persist SDP"
        );
        Ok(())
    })
    .await
    .unwrap();
    assert!(mock.closes.lock().await.is_empty());
    assert!(mock.revocations.lock().await.is_empty());
    assert_eq!(
        request(a, "leave", token, json!({})).await.0,
        StatusCode::NO_CONTENT
    );
    retry_backlog(b).await;
    assert_eq!(mock.revocations.lock().await.len(), 2);
}

#[tokio::test]
async fn rotation_replays_without_new_sessions_and_revokes_all_live_credentials() {
    let (s, mock) = state();
    exercise_rotation(&s, &s, &mock).await;
}

#[tokio::test]
async fn credential_mint_finishing_after_leave_is_revoked_not_committed() {
    let (s, mock) = state();
    let joined = joined(&s, "leaving during mint").await;
    let token = joined["token"].as_str().unwrap().to_owned();
    let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
    s.update(|r| {
        r.participants
            .get_mut(&id)
            .unwrap()
            .turn
            .as_mut()
            .unwrap()
            .issued = Timestamp::now() - Duration::from_secs(TURN_TTL / 2 + 1);
        Ok(())
    })
    .await
    .unwrap();
    mock.block_provision.store(true, Ordering::SeqCst);
    let copy = s.clone();
    let t = token.clone();
    let pending = tokio::spawn(async move {
        request(
            &copy,
            "turn",
            &t,
            json!({"generation":joined["turn"]["generation"]}),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if s.read(|r| Ok(r.participants[&id].turn_claim.is_some()))
                .await
                .unwrap()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        request(&s, "leave", &token, json!({})).await.0,
        StatusCode::NO_CONTENT
    );
    mock.block_provision.store(false, Ordering::SeqCst);
    assert_eq!(pending.await.unwrap().0, StatusCode::UNAUTHORIZED);
    retry_backlog(&s).await;
    assert_eq!(mock.revocations.lock().await.len(), 2);
    s.read(|r| {
        assert!(!r.participants.contains_key(&id));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn late_restart_completion_cannot_release_a_new_claim_or_resurrect_leave() {
    for leave in [false, true] {
        let (s, mock) = state();
        let joined = joined(&s, "late").await;
        let token = joined["token"].as_str().unwrap().to_owned();
        let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
        mock.block_restart.store(true, Ordering::SeqCst);
        let copy = s.clone();
        let t = token.clone();
        let body = restart_body(joined["turn"]["generation"].clone());
        let pending = tokio::spawn(async move { request(&copy, "restart-ice", &t, body).await });
        tokio::time::timeout(Duration::from_secs(1), mock.restart_started.notified())
            .await
            .unwrap();
        let newer = Uuid::new_v4();
        if leave {
            assert_eq!(
                request(&s, "leave", &token, json!({})).await.0,
                StatusCode::NO_CONTENT
            );
        } else {
            s.update(|r| {
                r.participants
                    .get_mut(&id)
                    .unwrap()
                    .restart
                    .as_mut()
                    .unwrap()
                    .claim = Some(Claim {
                    nonce: newer,
                    started: Timestamp::now(),
                });
                Ok(())
            })
            .await
            .unwrap();
        }
        mock.restart_resume.notify_one();
        let (status, _) = pending.await.unwrap();
        assert_eq!(
            status,
            if leave {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::CONFLICT
            }
        );
        s.read(|r| {
            if leave {
                assert!(!r.participants.contains_key(&id));
            } else {
                assert_eq!(
                    r.participants[&id]
                        .restart
                        .as_ref()
                        .unwrap()
                        .claim
                        .as_ref()
                        .unwrap()
                        .nonce,
                    newer
                );
            }
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn abandoned_restart_claim_recovers_without_expiring_call() {
    let (s, mock) = state();
    let joined = joined(&s, "abandoned").await;
    let token = joined["token"].as_str().unwrap();
    let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
    let body = restart_body(joined["turn"]["generation"].clone());
    mock.restart_unauthorized.store(true, Ordering::SeqCst);
    assert_eq!(
        request(&s, "restart-ice", token, body.clone()).await.0,
        StatusCode::BAD_GATEWAY
    );
    s.update(|r| {
        r.participants
            .get_mut(&id)
            .unwrap()
            .restart
            .as_mut()
            .unwrap()
            .claim = Some(Claim {
            nonce: Uuid::new_v4(),
            started: Timestamp::now() - Duration::from_secs(31),
        });
        Ok(())
    })
    .await
    .unwrap();
    expire_sessions(&s).await.unwrap();
    assert_eq!(
        request(&s, "snapshot", token, json!({})).await.0,
        StatusCode::OK
    );
    mock.restart_unauthorized.store(false, Ordering::SeqCst);
    assert_eq!(
        request(&s, "restart-ice", token, body).await.0,
        StatusCode::OK
    );
    assert!(mock.closes.lock().await.is_empty());
    assert!(mock.revocations.lock().await.is_empty());
}

#[tokio::test]
async fn restart_rejects_duplicate_mids_sendrecv_and_pending_negotiation() {
    let (s, _) = state();
    let joined = joined(&s, "validation").await;
    let token = joined["token"].as_str().unwrap();
    let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
    for suffix in [
        "a=mid:0\r\na=mid:1\r\na=recvonly\r\n",
        "a=mid:0\r\na=sendrecv\r\n",
    ] {
        let mut body = restart_body(joined["turn"]["generation"].clone());
        body["sessionDescription"]["sdp"] = json!(format!(
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n{suffix}"
        ));
        assert_eq!(
            request(&s, "restart-ice", token, body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    s.update(|r| {
        r.participants.get_mut(&id).unwrap().pending_offer = true;
        Ok(())
    })
    .await
    .unwrap();
    let result = request(
        &s,
        "restart-ice",
        token,
        restart_body(joined["turn"]["generation"].clone()),
    )
    .await;
    assert_eq!(result.0, StatusCode::CONFLICT);
    assert_eq!(result.1["code"], "ice_restart_pending");
}

#[tokio::test]
async fn expired_cached_password_is_pruned_without_removing_active_session() {
    let (s, _) = state();
    let joined = joined(&s, "expired credential").await;
    let id = Uuid::parse_str(joined["id"].as_str().unwrap()).unwrap();
    s.update(|r| {
        let cache = r.participants.get_mut(&id).unwrap().turn.as_mut().unwrap();
        cache.expires = Timestamp::now() - Duration::from_secs(1);
        cache.revoke_after = Timestamp::now() + Duration::from_secs(60);
        Ok(())
    })
    .await
    .unwrap();
    expire_sessions(&s).await.unwrap();
    s.read(|r| {
        let p = &r.participants[&id];
        assert!(p.turn.is_none());
        assert_eq!(
            p.turn_usernames.len(),
            1,
            "retain revocation name until conservative expiry"
        );
        assert_eq!(p.turn_retired.len(), 1);
        Ok(())
    })
    .await
    .unwrap();
}
