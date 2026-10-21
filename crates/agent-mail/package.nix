{
  lib,
  rustPlatform,
  deployment,
  correspondents,
}:
rustPlatform.buildRustPackage {
  pname = "agent-mail";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.cleanSource ../.;
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "agent-mail"
  ];
  cargoTestFlags = [
    "-p"
    "agent-mail"
  ];
  env = {
    AGENT_MAIL_AUTH_HOST = deployment.auth.host;
    AGENT_MAIL_CLIENT_ID = deployment.auth.agentMailClientId;
    AGENT_MAIL_JMAP_HOST = deployment.mail.host;
    AGENT_MAIL_DOMAIN = deployment.domain;
    AGENT_MAIL_AGENT_DOMAIN = deployment.mail.agents.domain;
    AGENT_MAIL_SHARED_LOCAL_PART = deployment.mail.agents.sharedLocalPart;
    AGENT_MAIL_CORRESPONDENTS = lib.concatStringsSep "," correspondents;
  };
  meta = {
    description = "Agent mail through Stalwart's JMAP, signed in with ZITADEL";
    mainProgram = "agent-mail";
  };
}
