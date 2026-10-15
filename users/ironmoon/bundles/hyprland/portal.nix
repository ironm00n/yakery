{ pkgs, ... }:
{
  screencopy = {
    allow_token_by_default = true;

    # Force Zoom to linear-only dmabuf by app_id; needs patched xdph + Zoom launched via zoom-scoped.
    force_linear_apps = (pkgs.callPackage ../../../../packages/zoom-scoped { }).appId;

    # TODO: try find a nicer picker
    # custom_picker_binary = pkgs.
  };
}
