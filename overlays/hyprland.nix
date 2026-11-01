final: prev: {
  # hyprland.fnl as a first-class config next to hyprland.lua: found first, compiled by
  # the bundled fennel, and its modules and macro modules watched for autoreload like .lua
  # ones; `require!` is the strict require. Local patch, never upstreamed.
  hyprland = prev.hyprland.overrideAttrs (old: {
    patches = old.patches or [ ] ++ [ ./hyprland-fennel.patch ];
    cmakeFlags = old.cmakeFlags ++ [
      (final.lib.cmakeFeature "HYPRLAND_FENNEL_DIR" "${final.lua5_5.pkgs.fennel}/share/lua/5.5")
    ];
  });
}
