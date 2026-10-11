# The Stalwart program accounts the scheduler signs in as, by local part, with all each may do:
# the core's, which it checks it holds before serving, and the one the feeds write with, named
# outside the core's `schedule-<key>` so no calendar's key can take it.
{ lib, calendars }:
{
  core = {
    schedule = [
      "authenticate"
      "jmapCalendarGet"
      "jmapCalendarEventCreate"
      "jmapPrincipalGetAvailability"
      "calendarSchedulingSend"
    ];
  }
  // lib.genAttrs (map (key: "schedule-${key}") calendars) (_: [
    "authenticate"
    "jmapPrincipalGetAvailability"
  ]);
  feed = {
    name = "calendar-feeds";
    permissions = [
      "authenticate"
      "davCalPropFind"
      "davCalGet"
      "davCalMultiGet"
      "davCalQuery"
      "davCalPut"
      "davCalDelete"
    ];
  };
}
