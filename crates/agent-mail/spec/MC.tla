---- MODULE MC ----
\* tlc -workers auto -config OneHost.cfg MC.tla (also TwoHosts.cfg and TwoHostsExclusive.cfg; TwoHostsOnce.cfg must fail)
EXTENDS AgentMail, TLC

CONSTANTS s1, s2, h1, h2, m1, m2, t1, t2

MCSessions == {s1, s2}
MCHosts == {h1, h2}
MCMails == {m1, m2}
MCThreads == {t1, t2}
MCInitThread == m1 :> t1 @@ m2 :> t2
\* s1 watches the topic both mails reach; s2 watches m1's thread.
MCWatch == s1 :> "topic" @@ s2 :> m1
OneHost == s1 :> h1 @@ s2 :> h1
TwoHosts == s1 :> h1 @@ s2 :> h2
====
