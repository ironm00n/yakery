{
  config,
  lib,
  my-lib,
  ...
}:
let
  cfg = config.bundles.outlook-sync;
  inherit (my-lib.ironmoon) domain schedule;
  accounts = import ../crates/scheduler/accounts.nix;
  address = account: "${account.name}@${domain}";
  calendar = "neu";
in
{
  imports = [ ../modules/outlook-sync.nix ];

  options.bundles.outlook-sync = {
    enable = lib.mkEnableOption "NEU's Outlook calendar mirrored into a calendar `${calendar}` in the owner's Stalwart account, from Graph change notifications at ${schedule.host}/${calendar}/";
    owner = lib.mkOption {
      type = lib.types.str;
      description = "The Stalwart address whose account holds the mirror, and whose NEU account signs in.";
    };
    write = lib.mkEnableOption "the owner's edits in `${calendar}` sent back to NEU's Outlook, once NEU grants the app Calendars.ReadWrite";
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = config.bundles.scheduler.enable;
        message = "bundles.outlook-sync serves on the scheduler's site and writes as its feeds account, so it needs bundles.scheduler";
      }
    ];

    services.outlook-sync = {
      enable = true;
      # "ironmoon calendar sync", publisher-verified as ironmoon LLC
      clientId = "c7bc1963-1ec6-484f-a8fe-b36e23f3e2b5";
      public = "https://${schedule.host}/${calendar}";
      inherit (config.services.scheduler) stalwart;
      account = address accounts.feed;
      appPassword = config.services.stalwart.appPasswords.${address accounts.feed}.file;
      calendar = my-lib.stalwart.calendarHref cfg.owner calendar;
      inherit (cfg) owner write;
    };

    services.stalwart.calendars.${cfg.owner}.${calendar}.shares = {
      ${address accounts.feed} = [
        "mayReadItems"
        "mayWriteAll"
      ];
      ${address accounts.core} = [ "mayReadFreeBusy" ];
    };

    # its own location, so Graph's bursts meet the site's rate and not the booking form's
    bundles.reverse-proxy.hosts.${schedule.host}.locations."/${calendar}/" = {
      inherit (config.services.outlook-sync) port;
      extraConfig = "limit_req zone=schedule burst=40 nodelay;";
    };
  };
}
