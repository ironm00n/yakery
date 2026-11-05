# The Stalwart program accounts the scheduler signs in as, by local part, with all each may do:
# the core's, which it checks it holds no more than before serving, and the feeds'.
{
  core = {
    name = "schedule";
    permissions = [
      "authenticate"
      "jmapCalendarGet"
      "jmapCalendarEventCreate"
      "jmapPrincipalGetAvailability"
      "jmapPrincipalQuery"
      "calendarSchedulingSend"
    ];
  };
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
      # outlook-sync reads the owner's edits in its mirror as what changed since its last look
      "davSyncCollection"
    ];
  };
}
