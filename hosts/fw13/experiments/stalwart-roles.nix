# perms : { string : string }
# defaults : { user : string[]; group : string[]; tenant : string[]; superuser : string[] }
# output : id : { name : string; extends string[]; enabled : string[]; disabled : string[] }
{ perms, defaults }:
{
  default-user = {
    name = "User (Default)";
    enabled = defaults.user;
  };
  default-group = {
    name = "Group (Default)";
    enabled = defaults.group;
  };
  default-tenant ={
    name = "Tenant Admin (Default)";
    enabled = defaults.tenant;
  };
  default-superuser = {
    name = "Admin (Default)";
    enabled = defaults.superuser;
  };

  member = {
    name = "Member";
    enabled = with perms; [
      authenticate authenticateWithAlias
    ];
  };

  calendar = {
    name = "Calendar";
    enabled = with perms; [
      calendarAlarmsSend calendarSchedulingSend calendarSchedulingReceive 
    ];
  };

  no-limit = {
    name = "No Limits";
    enabled = with perms; [ unlimitedRequests unlimitedUploads ];
  };

  admin = {
    name = "Administrator";
    # extends = [ "member" "calendar" "no-limit" ];
    enabled = [ ];
  };

  super-admin = {
    name = "Super Administrator";
    # extends = [ "admin" ];
    # enabled = with perms; [ impersonate fetchAnyBlob ];
    enabled = builtins.attrValues perms;
  };
}
