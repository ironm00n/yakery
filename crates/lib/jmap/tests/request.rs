use serde_json::json;

use super::*;
use crate::{Changed, Got, MAIL, Queried};

#[test]
fn a_request_numbers_its_calls_in_order() {
    let mut request = Request::new(&[crate::CORE, MAIL]);
    request.call("Email/get", json!({ "ids": ["m1"], "accountId": "a" }));
    request.call("Email/query", json!({}));
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"methodCalls":[["Email/get",{"accountId":"a","ids":["m1"]},"0"],["Email/query",{},"1"]],"using":["urn:ietf:params:jmap:core","urn:ietf:params:jmap:mail"]}"#
    );
}

#[test]
fn each_call_reads_the_answer_under_its_id_and_its_name() {
    let mut request = Request::new(&[MAIL]);
    let [get, query, set, changes, lost] =
        ["Email/get", "Email/query", "Email/set", "Email/changes", "Email/import"]
            .map(|method| request.call(method, json!({})));
    let response: Response = serde_json::from_str(
        r#"{"methodResponses": [
            ["Email/query", {"ids": ["m2"]}, "1"],
            ["Email/get", {"state": "s1", "list": [{"id": "m1"}]}, "0"],
            ["Email/changes", {"created": [1]}, "3"],
            ["error", {"type": "stateMismatch", "description": "s0 is gone"}, "2"],
            ["Email/set", {"newState": "s2"}, "2"],
            ["Email/get", {"list": []}, "4"]
        ]}"#,
    )
    .unwrap();
    let got: Got<Value> = response.answer(get).unwrap();
    assert_eq!((got.state.as_str(), got.list), ("s1", vec![json!({ "id": "m1" })]));
    assert_eq!(response.answer::<Queried>(query).unwrap().ids, ["m2"]);
    let Err(Failed::Refused(refused)) = response.answer::<Value>(set) else { panic!() };
    assert_eq!(
        (refused.kind.as_str(), refused.to_string()),
        ("stateMismatch", "stateMismatch: s0 is gone".into())
    );
    let unreadable = response.answer::<Changed>(changes).unwrap_err();
    assert_eq!(
        unreadable.to_string(),
        "its answer doesn't read: invalid type: integer `1`, expected a string"
    );
    assert!(matches!(response.answer::<Value>(lost), Err(Failed::Unanswered)));
}

#[test]
fn a_method_error_without_its_type_or_a_response_without_answers_still_reads() {
    let mut request = Request::new(&[MAIL]);
    let call = request.call("Email/get", json!({}));
    let response: Response =
        serde_json::from_str(r#"{"methodResponses":[["error",null,"0"]]}"#).unwrap();
    let Err(Failed::Refused(refused)) = response.answer::<Value>(call) else { panic!() };
    assert_eq!((refused.kind.as_str(), refused.description), ("", None));
    for empty in ["{}", r#"{"methodResponses":null}"#] {
        let response: Response = serde_json::from_str(empty).unwrap();
        assert_eq!(
            response.answer::<Value>(call).unwrap_err().to_string(),
            "the response holds no answer to it"
        );
    }
}
