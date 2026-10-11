use pikker::{
    domain::Role,
    meta::{decode::*, model::*, opgg::*},
};
use serde_json::json;

fn catalog() -> Vec<ChampionIdentity> {
    vec![ChampionIdentity {
        id: 25,
        api_name: "MORGANA".into(),
        display_name: "Morgana".into(),
    }]
}

#[test]
fn general_contains_all_five_roles_and_counts_override_rounded_rate() {
    let row = json!({"champion":"Morgana","play":3,"win":2,"win_rate":0.67});
    let input = json!({"data":{"positions":{"top":[row.clone()],"jungle":[row.clone()],
        "mid":[row.clone()],"adc":[row.clone()],"support":[row]}}});
    let output = parse_general(&input, catalog(), Some("gold".into()), 1000).unwrap();
    assert_eq!(output.stats.len(), 5);
    assert!(output.stats.iter().any(|r| r.role == Role::Bot));
    assert!((output.stats[0].observed_rate() - 2.0 / 3.0).abs() < 0.00001);
    assert!(
        output
            .stats
            .iter()
            .all(|r| r.scope == SourceScope::default())
    );
}

#[test]
fn requested_gold_is_not_verified_gold_and_locale_is_not_region() {
    let input = json!({"lang":"pt_BR","position":"SUPPORT","data":{
        "summary":{"positions":[{"stats":{"play":100,"win_rate":0.52}}]},
        "trends":{"win":{"version":"16.20","created_at":"2026-10-10T23:07:41+09:00","rate":0.52}}
    }});
    let row = parse_details(&input, &catalog()[0], Role::Support, Some("gold"), 1000).unwrap();
    assert_eq!(row.requested_rank.as_deref(), Some("gold"));
    assert_eq!(row.scope.region, None);
    assert_eq!(row.scope.verified_rank, None);
    assert_eq!(row.scope.patch, None); // Trend is not proof of the population of builds.
    assert_eq!(
        row.scope.provider_timestamp.as_deref(),
        Some("2026-10-10T23:07:41+09:00")
    );
}

#[test]
fn runes_and_spells_preserve_ids_and_names() {
    let input = json!({"data":{
        "runes":{"primary_rune_ids":[8214,8226],"primary_rune_names":["Aery","Manaflow"],
            "secondary_rune_ids":[8345],"secondary_rune_names":["Biscuits"],"stat_mod_ids":[5008],
            "play":10,"win":6},
        "summoner_spells":{"ids":[4,14],"ids_names":["Flash","Ignite"],"play":10,"win":6}
    }});
    let row = parse_details(&input, &catalog()[0], Role::Support, None, 1000).unwrap();
    assert_eq!(row.spells[0].ids, vec![4, 14]);
    assert_eq!(row.spells[0].names, vec!["Flash", "Ignite"]);
    assert_eq!(row.runes[0].ids, vec![8214, 8226, 8345, 5008]);
    assert_eq!(row.spells[0].games, Some(10));
}

#[test]
fn numeric_spell_name_placeholders_keep_ids_without_fabricated_names() {
    let input = json!({"data":{"summoner_spells":{"ids":[4,7],"ids_names":[4,7],"win":13086,"play":24666}}});
    let details = parse_details(&input, &catalog()[0], Role::Support, Some("gold"), 1000).unwrap();
    assert_eq!(details.spells[0].ids, vec![4, 7]);
    assert!(details.spells[0].names.is_empty());
    assert_eq!(details.spells[0].wins, Some(13086));
}

#[test]
fn official_static_labels_resolve_spell_and_rune_ids() {
    let summoner =
        json!({"data":{"Flash":{"key":"4","name":"Flash"},"Heal":{"key":"7","name":"Curar"}}});
    let runes = json!([{"id":8200,"name":"Feitiçaria","slots":[{"runes":[{"id":8214,"name":"Invocar Aery"}]}]}]);
    let labels = parse_labels(&summoner, &runes, "16.20.1").unwrap();
    assert_eq!(labels.spells.get(&7).map(String::as_str), Some("Curar"));
    assert_eq!(
        labels.runes.get(&8214).map(String::as_str),
        Some("Invocar Aery")
    );
}

#[test]
fn synergy_keeps_partner_id_and_role_instead_of_selected_champion() {
    let input = json!({"data":{"synergies":{"adc":[{"champion_id":267,"synergy_champion_id":222,"play":100,"win":60,"win_rate":0.6}]}}});
    let row = parse_details(&input, &catalog()[0], Role::Support, None, 1000).unwrap();
    assert_eq!(row.synergies[0].champion_id, 222);
    assert_eq!(row.synergies[0].role, Some(Role::Bot));
}

#[test]
fn json_and_sse_envelopes_select_matching_response_not_notification() {
    let direct = r#"{"jsonrpc":"2.0","id":3,"result":{"structuredContent":{"data":{"ok":true}}}}"#;
    assert_eq!(decode_envelope(direct, 3).unwrap()["data"]["ok"], true);
    let sse = format!("event: message\ndata: {{\"method\":\"notice\"}}\n\ndata: {direct}\n\n");
    assert_eq!(decode_envelope(&sse, 3).unwrap()["data"]["ok"], true);
    assert!(decode_envelope(direct, 4).is_err());
}

#[test]
fn observed_constructor_format_is_parsed_without_execution() {
    let text = "class Result: data\nclass Row: champion,play,win\n\nResult([Row(\"Nami\",10,6)])";
    assert_eq!(
        decode_opgg(text).unwrap(),
        json!({"data":[{"champion":"Nami","play":10,"win":6}]})
    );
    let booleans = "class Result: enabled,value\n\nResult(false,null)";
    assert_eq!(
        decode_opgg(booleans).unwrap(),
        json!({"enabled":false,"value":null})
    );
}

#[test]
fn observed_provider_diagnostics_with_underscore_fields_are_data_not_code() {
    let text = "class Result: data,_field_diagnostics\nclass Diagnostics: unmatched_fields,hint\n\nResult([],Diagnostics([\"data.synergies.support[].play\"],\"Field missing\"))";
    let result = decode_opgg(text).unwrap();
    assert_eq!(
        result["_field_diagnostics"]["unmatched_fields"][0],
        "data.synergies.support[].play"
    );
}

#[test]
fn malicious_text_unknown_shape_and_oversize_fail_closed() {
    for expression in [
        "__import__(\"os\").system(\"anything\")",
        "Row(name=\"Nami\")",
        "Row(1,2)",
        "Other(1)",
        "Row(1);anything()",
    ] {
        assert!(decode_opgg(&format!("class Row: name\n\n{expression}")).is_err());
    }
    assert!(decode_opgg(&"x".repeat(2 * 1024 * 1024 + 1)).is_err());
    assert!(parse_general(&json!({"data":{}}), catalog(), None, 1000).is_err());
}

#[test]
fn response_429_exposes_retry_after_without_payload_and_redirects_are_errors() {
    let response = HttpResponse {
        status: 429,
        headers: vec![("Retry-After".into(), "17".into())],
        body: b"secret remote body".to_vec(),
    };
    match checked_body(response).unwrap_err() {
        pikker::runtime::IntegrationError::Http {
            status,
            retry_after,
        } => {
            assert_eq!(status, 429);
            assert_eq!(retry_after.unwrap().as_secs(), 17);
        }
        other => panic!("{other}"),
    }
    assert!(
        checked_body(HttpResponse {
            status: 302,
            headers: vec![],
            body: vec![]
        })
        .is_err()
    );
}

#[test]
fn retry_after_accepts_http_date_as_well_as_seconds() {
    assert_eq!(
        retry_after_delay("Thu, 01 Jan 1970 00:00:17 GMT", std::time::UNIX_EPOCH),
        Some(std::time::Duration::from_secs(17))
    );
    assert_eq!(
        retry_after_delay("17", std::time::UNIX_EPOCH),
        Some(std::time::Duration::from_secs(17))
    );
    assert_eq!(retry_after_delay("invalid", std::time::UNIX_EPOCH), None);
}

#[test]
fn catalog_normalizes_names_and_rejects_duplicate_ids() {
    let input = json!({"data":{"Morgana":{"key":"25","id":"Morgana","name":"Morgana"},
        "KaiSa":{"key":"145","id":"KaiSa","name":"Kai'Sa"}}});
    let rows = parse_catalog(&input).unwrap();
    assert_eq!(
        rows.iter().find(|c| c.id == 145).unwrap().api_name,
        "KAI_SA"
    );
    let bad =
        json!({"data":{"A":{"key":"25","id":"A","name":"A"},"B":{"key":"25","id":"B","name":"B"}}});
    assert!(parse_catalog(&bad).is_err());
}

#[test]
fn provider_checks_available_tools_before_fetching_statistics() {
    use std::sync::{Arc, Mutex};
    struct Transport(Arc<Mutex<Vec<String>>>);
    impl HttpTransport for Transport {
        fn send(
            &self,
            request: HttpRequest,
        ) -> Result<HttpResponse, pikker::runtime::IntegrationError> {
            let body = request.body.unwrap();
            let method = body["method"].as_str().unwrap().to_owned();
            self.0.lock().unwrap().push(method.clone());
            let result = if method == "initialize" {
                json!({"protocolVersion":"2024-11-05"})
            } else {
                json!({"tools":[]})
            };
            let bytes = if body.get("id").is_none() {
                vec![]
            } else {
                serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                    .unwrap()
            };
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: bytes,
            })
        }
    }
    let calls = Arc::new(Mutex::new(vec![]));
    let mut provider = OpggProvider::new(Transport(calls.clone()), catalog(), None);
    assert!(provider.fetch_general().is_err());
    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|m| m == "tools/list"));
    assert!(!calls.iter().any(|m| m == "tools/call"));
}
