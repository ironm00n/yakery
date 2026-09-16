{ pkgs }:
let
  # graphics=false makes the serial port /dev/console, which is where the
  # status lines go and what the driver captures; test-instrumentation then
  # turns ShowStatus off to keep that capture quiet.
  common =
    { lib, ... }:
    {
      virtualisation.graphics = false;
      systemd.settings.Manager.ShowStatus = lib.mkForce true;
    };
in
pkgs.testers.runNixOSTest {
  name = "ok-color-bundle";

  nodes = {
    patched = {
      imports = [
        common
        ../../bundles/ok-color.nix
      ];
      bundles.ok-color.enable = true;
    };
    stock = common;
  };

  testScript = ''
    def ok(sgr):
        return f"[\x1b[0;{sgr}m  OK  \x1b[0m]"

    for m in [patched, stock]:
        m.wait_for_unit("multi-user.target")

    with subtest("the bundle turns the OK blue"):
        console = patched.get_console_log()
        assert ok("1;34") in console, "no blue OK on the console"
        assert ok(32) not in console, "a green OK is still on the console"

    with subtest("nixpkgs' systemd still defaults to green"):
        assert ok(32) in stock.get_console_log(), "no green OK on the console"
  '';
}
