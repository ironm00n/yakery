{
  lib,
  deployment,
  correspondents,
  correspondentsFor,
  periods ? import ./periods.nix,
}:
{
  AGENT_MAIL_AUTH_HOST = deployment.auth.host;
  AGENT_MAIL_CLIENT_ID = deployment.auth.agentMailClientId;
  AGENT_MAIL_JMAP_HOST = deployment.mail.host;
  AGENT_MAIL_DOMAIN = deployment.domain;
  AGENT_MAIL_AGENT_DOMAIN = deployment.mail.agents.domain;
  AGENT_MAIL_SHARED_LOCAL_PART = deployment.mail.agents.sharedLocalPart;
  AGENT_MAIL_CORRESPONDENTS = lib.concatStringsSep "," correspondents;
  AGENT_MAIL_CORRESPONDENTS_FOR = builtins.toJSON correspondentsFor;
  AGENT_MAIL_RECHECK_SECS = toString periods.recheck;
  AGENT_MAIL_BACKOFF_SECS = toString periods.backoff;
  AGENT_MAIL_STALE_SECS = toString periods.stale;
}
