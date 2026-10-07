# Run Zoom in an app-<appId>-* systemd scope (even when running from terminal) so the portal
# reports that app_id, which `force_linear_apps` matches.
{
  symlinkJoin,
  writeShellScriptBin,
  systemd,
  zoom-us,
}:
let
  appId = "Zoom";
in
symlinkJoin {
  name = "zoom-us-app-scoped";
  paths = [
    (writeShellScriptBin "zoom" ''
      exec ${systemd}/bin/systemd-run --user --scope --unit="app-${appId}-$RANDOM" ${zoom-us}/bin/zoom "$@"
    '')
    zoom-us
  ];
  postBuild = ''
    if [ ! -e "$out/share/applications/${appId}.desktop" ]; then
      echo "zoom-scoped: ${appId}.desktop is missing; xdg-desktop-portal >= 1.21 drops a scope's app_id without a matching desktop file" >&2
      exit 1
    fi
  '';
  passthru = { inherit appId; };
}
