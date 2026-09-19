# Clipboard between host and guest over vsock, one selection per keypress, in both directions.
# Both ends live here so the addresses exist in one place: the qemu device and the guest's
# listeners, and the two host scripts the hyprland bundle binds.
#
# Neither direction mirrors. Push sends what is on the host clipboard right now; pull asks the
# guest for what is on its clipboard right now. So the guest never sees what you copy while it
# runs unless you send it, and nothing from the guest reaches the host clipboard unless you ask
# — which is the grant that matters, since anything in the guest can set its selection. Text only.
#
# Not spice-vdagent (qemu ≥ 6.1 can speak vdagent to its own GTK window without SPICE): nixpkgs
# builds it Xlib-only, and wlroots refuses to bridge an X11 selection either way unless an
# Xwayland surface is focused (xwayland/selection/{incoming,outgoing}.c). The guest's editor is
# native Wayland, so one never is. wl-clipboard uses wlr-data-control and needs no focus.
{ pkgs }:
let
  cid = 3;
  ports = {
    push = 7777;
    pull = 7778;
  };
  clip =
    name: text:
    pkgs.writeShellApplication {
      inherit name text;
      runtimeInputs = with pkgs; [
        socat
        wl-clipboard
      ];
    };
in
{
  device = "vhost-vsock-pci,guest-cid=${toString cid}";

  # One script rather than two sway exec lines: sway splits an unquoted exec on the commas in a
  # socat address, and both listeners want the session's WAYLAND_DISPLAY anyway.
  serve = clip "hackathon-vm-clip-serve" ''
    socat -u VSOCK-LISTEN:${toString ports.push},fork EXEC:wl-copy &
    exec socat VSOCK-LISTEN:${toString ports.pull},fork EXEC:"wl-paste --no-newline"
  '';

  push = clip "hackathon-vm-clip-push" ''
    wl-paste --no-newline | socat -u - VSOCK-CONNECT:${toString cid}:${toString ports.push}
  '';

  pull = clip "hackathon-vm-clip-pull" ''
    socat -u VSOCK-CONNECT:${toString cid}:${toString ports.pull} - | wl-copy
  '';
}
