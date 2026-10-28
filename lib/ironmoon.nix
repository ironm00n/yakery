rec {
  domain = "ironmoon.dev";
  auth = {
    host = "auth.${domain}";
    agentMailClientId = "395450175197396579";
  };
  mail = {
    host = "mail.${domain}";
    # the submission signer's public key, which agent-mail checks each Submission-Signature with
    submissionKey = null;
    agents = {
      domain = "agent.${domain}";
      sharedLocalPart = "agent";
    };
  };
}
