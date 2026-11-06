use super::*;

#[test]
fn an_authoritys_endpoints_are_under_its_v2_path() {
    let at = endpoints(MICROSOFT, "organizations").unwrap();
    let base = "https://login.microsoftonline.com/organizations/oauth2/v2.0";
    assert_eq!(at.authorization.to_string(), format!("{base}/authorize"));
    assert_eq!(at.token.to_string(), format!("{base}/token"));
    assert_eq!(at.device.unwrap().to_string(), format!("{base}/devicecode"));
    assert!(at.userinfo.is_none());
}

#[test]
fn a_stand_ins_origin_keeps_no_trailing_slash() {
    let at = endpoints("http://127.0.0.1:9/", "t").unwrap();
    assert_eq!(at.token.to_string(), "http://127.0.0.1:9/t/oauth2/v2.0/token");
}
