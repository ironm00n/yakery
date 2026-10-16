{ lib }:
rec {
  # Stalwart represents arrays as objects keyed by stringified indices.
  # Recursively convert lists to that shape so configs use normal Nix lists.
  listsToIndexedObjects =
    value:
    if builtins.isList value then
      builtins.listToAttrs (
        lib.imap0 (i: v: {
          name = toString i;
          value = listsToIndexedObjects v;
        }) value
      )
    else if builtins.isAttrs value && !lib.isDerivation value then
      builtins.mapAttrs (_: listsToIndexedObjects) value
    else
      value;

  create = object: records: {
    "@type" = "create";
    inherit object;
    value = listsToIndexedObjects records;
  };
  create1 = object: id: body: create object { ${id} = body; };

  update = object: patch: {
    "@type" = "update";
    inherit object;
    value = listsToIndexedObjects patch;
  };

  destroyAll = object: {
    "@type" = "destroy";
    inherit object;
  };
  destroyWhere = object: filter: {
    "@type" = "destroy";
    inherit object;
    value = listsToIndexedObjects filter;
  };

  upsert = object: value: {
    "@type" = "upsert";
    inherit object;
    value = listsToIndexedObjects value;
  };
  upsert1 = object: id: body: upsert object { ${id} = body; };
  upsertOn = object: matchOn: value: {
    "@type" = "upsert";
    inherit object matchOn;
    value = listsToIndexedObjects value;
  };
  upsertOn1 = object: id: matchOn: body: upsertOn object matchOn { ${id} = body; };

  reconcile = object: matchOn: value: {
    "@type" = "reconcile";
    inherit object matchOn;
    value = listsToIndexedObjects value;
  };

  ref = id: "#${id}";
  toSet = names: lib.genAttrs names (_: true);

  fileSecret = path: {
    "@type" = "File";
    filePath = toString path;
  };
}
