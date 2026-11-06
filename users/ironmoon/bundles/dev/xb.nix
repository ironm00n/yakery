{ runCommandLocal, homeDirectory }:
# an xb deploy moves the gc root ~/.local/state/xb/current, so PATH reaches the deployed client without a switch
runCommandLocal "xb" { meta.mainProgram = "xb"; } ''
  mkdir -p $out/bin
  ln -s ${homeDirectory}/.local/state/xb/current/bin/xb $out/bin/xb
''
