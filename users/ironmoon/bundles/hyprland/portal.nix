{ pkgs, ... }:
{
  screencopy = {
    allow_token_by_default = true;

    # Force Zoom to linear-only dmabuf by app_id; needs patched xdph + zoom-scoped (user.nix).
    force_linear_apps = "us.zoom.Zoom";

    # TODO: try find a nicer picker
    # custom_picker_binary = pkgs.
  };
}
