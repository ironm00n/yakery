# agent-mail as ironmoon.dev runs it, for the claude bundle's PATH and the mail-listener mod the tap trusts alike
{
  callPackage,
  my-lib,
  secrets,
}:
callPackage ./package.nix {
  deployment = my-lib.ironmoon;
  correspondents = secrets.data.mail.agentCorrespondents;
  correspondentsFor = secrets.data.mail.agentCorrespondentsFor;
}
