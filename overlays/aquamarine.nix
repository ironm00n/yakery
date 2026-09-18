final: prev: {
  # aquamarine 0.15.0 (async commits, #363) rejects the disable commit for a
  # connector that just went away, so its CRTC stays active in the kernel and
  # the re-plugged output fails every modeset (0x0@60, black). Upstream issue
  # hyprwm/aquamarine#386; fix is PR #395, auto-closed under the vouch policy.
  # Drop once a release with an equivalent lands in nixos-unstable.
  aquamarine = prev.aquamarine.overrideAttrs (old: {
    patches = old.patches or [ ] ++ [
      (final.fetchpatch {
        name = "aquamarine-disable-kms-before-disconnect.patch";
        url = "https://github.com/hyprwm/aquamarine/commit/d6c3c8a393627e518cde7ede43edba563b225490.patch";
        hash = "sha256-8VVM8DfjKwptw8wW0PnvHf6/iwd5olzE40IwFkePmPA=";
      })
    ];
  });
}
