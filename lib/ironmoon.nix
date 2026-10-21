rec {
  domain = "ironmoon.dev";
  auth = {
    host = "auth.${domain}";
    agentMailClientId = "395450175197396579";
  };
  mail = {
    host = "mail.${domain}";
    agents = {
      domain = "agent.${domain}";
      sharedLocalPart = "agent";
    };
  };
}
