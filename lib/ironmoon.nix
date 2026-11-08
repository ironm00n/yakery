rec {
  domain = "ironmoon.dev";
  auth = {
    host = "auth.${domain}";
    agentMailClientId = "395450175197396579";
  };
  mail = {
    host = "mail.${domain}";
    # Stalwart's HTTP listener, which only programs on its host reach
    loopback = {
      address = "127.0.0.1";
      port = 8080;
    };
    # the submission signer's public key, which agent-mail checks each Submission-Signature with
    submissionKey = "l9I3tCq1ru6k2Yb2/RfrnaxqdUmglcIeMW8vurYRRWQ";
    # by program account, the alias of its own it mails from
    senders = {
      forgejo = "code";
      zitadel = "auth";
    };
    agents = {
      domain = "agent.${domain}";
      sharedLocalPart = "agent";
      # whom its commits name, in each account's agent scope
      identity = {
        name = "ironmoon's agent";
        email = "${mail.agents.sharedLocalPart}@${domain}";
      };
    };
  };
  schedule = {
    host = "schedule.${domain}";
    timeZone = "America/New_York";
  };
}
