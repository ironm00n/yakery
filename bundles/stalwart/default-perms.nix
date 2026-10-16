# Port of Stalwart's DefaultPermissions::default()
# (stalwart source: crates/common/src/auth/permissions.rs).
#
# Buckets every permission into the user / group / tenant / superuser sets by
# name -- the same classification the server uses to seed its four built-in
# roles. Driven by `perms` (the package's permission registry), so the result
# tracks the deployed Stalwart version instead of freezing at first boot the
# way the server's bootstrapped roles do.
#
# Keep the classification below in sync with DefaultPermissions::default().
{ lib, perms }:
let
  inherit (lib)
    hasPrefix
    hasInfix
    any
    filter
    elem
    ;

  anyPrefix = prefixes: name: any (p: hasPrefix p name) prefixes;

  # A permission's wire name -> the buckets it belongs to.
  bucketsOf =
    name:
    if elem name [ "authenticate" "authenticateWithAlias" "interactAi" ] then
      [ "user" "tenant" "superuser" ]
    else if
      elem name [ "impersonate" "unlimitedRequests" "unlimitedUploads" "liveMetrics" "liveTracing" ]
    then
      [ "superuser" ]
    else if elem name [ "fetchAnyBlob" "liveDeliveryTest" ] then
      [ "tenant" "superuser" ]
    else if anyPrefix [ "jmap" "imap" "pop3" "calendar" "email" "dav" "sieve" ] name then
      [ "user" "group" ]
    else if
      anyPrefix [ "sysMaskedEmail" "sysArchivedItem" "sysAccountSettings" "sysPublicKey" ] name
      || (hasPrefix "sysSpamTrainingSample" name && !(hasInfix "Create" name))
    then
      [ "user" "group" "superuser" ]
    else if anyPrefix [ "sysAccountPassword" "sysApiKey" "sysAppPassword" ] name then
      [ "user" "superuser" ]
    else if
      anyPrefix [
        "sysDomain"
        "sysDkimSignature"
        "sysAcmeProvider"
        "sysAccount"
        "sysRole"
        "sysOAuthClient"
        "sysMailingList"
        "sysExternalReport"
        "sysDnsServer"
        "sysQueuedMessage"
      ] name
    then
      [ "tenant" "superuser" ]
    else
      [ "superuser" ];

  inBucket = bucket: filter (name: elem bucket (bucketsOf name)) (builtins.attrNames perms);
in
{
  user = inBucket "user";
  group = inBucket "group";
  tenant = inBucket "tenant";
  superuser = inBucket "superuser";
}
