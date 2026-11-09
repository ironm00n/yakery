# A wait's periods in seconds: how long it waits for a push before scanning anyway, its first pause before signing in
# again, how long it goes without a finished scan before failing, and the spread its reopening of a dropped push stream
# is drawn from. The VM test builds with shorter ones.
{
  recheck = 150;
  backoff = 5;
  stale = 300;
  reopen = 30;
}
