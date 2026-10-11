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
    agents = {
      domain = "agent.${domain}";
      sharedLocalPart = "agent";
    };
  };
  schedule = {
    host = "schedule.${domain}";
    timeZone = "America/New_York";
  };
}
