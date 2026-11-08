# claude-agent-adopt-gpg-key HOMEDIR FINGERPRINT, run by him: copies the agent's secret key FINGERPRINT from his HOMEDIR
# into the agent account by a pipe, in place of every key the account holds. Every check but the last runs before
# either keyring changes; restart claude-agent-gpg-key once it succeeds.
homedir=$1
fpr=$2
agent=(sudo -n -u claude-agent "$(command -v gpg)" --batch)
die() {
  echo "$*" >&2
  exit 1
}
mine() { gpg --homedir "$homedir" --batch "$@"; }
# a designated revoker's rvk record comes between a key's sec and its fpr
primaries() { awk -F: '$1 == "sec" { primary = 1 } $1 == "fpr" && primary { print $10; primary = 0 }'; }

listing=$(mine --with-colons --list-secret-keys "$fpr") || die "$homedir holds no secret key $fpr"
[ "$(primaries <<<"$listing")" = "$fpr" ] || die "$fpr isn't the whole fingerprint of one primary key in $homedir"
awk -F: -v uid="<$email>" '$1 == "uid" && $2 !~ /^[eir]$/ && substr($10, length($10) - length(uid) + 1) == uid { found = 1 }
  END { exit !found }' <<<"$listing" || die "$fpr has no valid uid with <$email>"
awk -F: '$1 == "grp" { print $10 }' <<<"$listing" | while read -r grip; do
  case $(gpg-connect-agent --homedir "$homedir" "keyinfo $grip" /bye | awk '$1 == "S" { print $8 }') in
  C) ;;
  P) die "$fpr needs a passphrase, which the agent's commits can't give" ;;
  *) die "$homedir doesn't hold every secret part of $fpr" ;;
  esac
done

mine --export-secret-keys "$fpr" | "${agent[@]}" --import
"${agent[@]}" --with-colons --list-secret-keys | primaries | { grep -vx "$fpr" || true; } |
  xargs -r "${agent[@]}" --yes --delete-secret-and-public-key
[ "$("${agent[@]}" --with-colons --list-secret-keys | primaries)" = "$fpr" ] || die "the agent account doesn't hold $fpr alone"
