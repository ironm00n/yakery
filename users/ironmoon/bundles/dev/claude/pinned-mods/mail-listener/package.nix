{
  lib,
  runCommand,
  agent-mail,
}:
runCommand "claude-mod-mail-listener" { } ''
  cp -r ${./plugin} $out
  chmod -R u+w $out
  substituteInPlace $out/hooks/register.js --replace-fail '@agentMail@' '${lib.getExe agent-mail}'
''
