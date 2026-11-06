use super::*;

fn sign_in() -> Pkce {
    Pkce::kept(Secret::new("s1"), Secret::new("v"))
}

#[test]
fn a_code_comes_back_with_its_state() {
    let returned = Returned::read("code=the%2Bcode&state=s1");
    assert_eq!(returned.state(), Some("s1"));
    assert_eq!(returned.code(&sign_in()).map(|c| c.code), Ok("the+code".into()));
}

#[test]
fn an_error_wins_over_a_code_and_keeps_its_description() {
    let returned =
        Returned::read("error=access_denied&error_description=no+thanks&code=c&state=s1");
    let denied =
        Denied::Error { error: "access_denied".into(), description: Some("no thanks".into()) };
    assert_eq!(returned.code(&sign_in()).map(|c| c.code), Err(denied));
}

#[test]
fn neither_is_codeless() {
    assert_eq!(Returned::read("state=s1").code(&sign_in()).map(|c| c.code), Err(Denied::Codeless));
}

#[test]
fn another_state_or_none_gives_no_code_and_no_error() {
    for query in ["code=c&state=s2", "code=c", "error=access_denied&state=s2", ""] {
        assert_eq!(
            Returned::read(query).code(&sign_in()).map(|c| c.code),
            Err(Denied::Mismatched),
            "{query}"
        );
    }
}

#[test]
fn a_return_comes_from_the_sign_in_whose_state_it_carries() {
    let returns_from = |query| Returned::read(query).returns_from(&sign_in());
    assert_eq!(
        ["code=c&state=s1", "code=c&state=s2", "code=c"].map(returns_from),
        [true, false, false]
    );
}
