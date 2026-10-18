---------------------------- MODULE AgentMail ----------------------------
(* The mail-state protocol of agent-mail's wait, reply and done, against
   Stalwart as observed in its 0.16.26 source:
   - Email/set's ifInState is checked against state read at request start,
     so it doesn't serialise claims; only the host-local lock does.
   - Threads merge asynchronously; a thread watch follows its start email's
     current thread.
   - A submission exists even when every recipient refused it.
   - A mark can land on the server while its reply is lost.
   Only inbound mail is modelled, and every mail asks for a receipt.
   Every check reads the whole inbox, where agent-mail reads Email/changes
   after its first. That finds the same due mail: the state it reads changes
   from moves only once a whole check has succeeded, and the mail that check
   found due is read again, as is all mail of a watched thread, so other mail
   is unchanged since a check that did not find it due. Seen mail is reminded
   of once, so that check suffices for it. A mail has one id here; agent-mail
   maps the old ids of a merged thread to the current one before it claims. *)
EXTENDS Naturals, FiniteSets

CONSTANTS Sessions, Hosts, HostOf, Mails, Threads, InitThread, Watch, MaxCrashes

Keywords == {"seen", "answered", "handled", "mdnsent"}
\* A refused or unknown outcome changes nothing, so it is a stutter.
Outcomes == {"delivered", "partial"}
None == "none"

VARIABLES
    arrived,   \* inbound mail in the inbox
    thread,    \* each mail's current thread
    kw,        \* each mail's keywords
    replied,   \* mail an agent-mail reply reached at least one recipient for
    othered,   \* mail a reply from another tool references by In-Reply-To
    reused,    \* mail whose Message-ID another inbox mail of its thread also carries
    pc,        \* each session's step in a wait
    snap,      \* what a session's claim read
    lock,      \* each host's claim lock
    printed,   \* every mail each session has printed, ever
    reminded,  \* every mail each session has been reminded of, ever
    receipts,  \* receipts sent per mail
    crashes    \* crashes so far, bounded so that liveness can hold

mail == <<arrived, thread, kw, replied, othered, reused, receipts>>
session == <<pc, snap, lock, printed, reminded, crashes>>
vars == <<mail, session>>

Watched(s) ==
    IF Watch[s] = "topic" THEN arrived
    ELSE {m \in arrived : Watch[s] \in arrived /\ thread[m] = thread[Watch[s]]}

\* agent-mail's own replies are tagged `$agentmail` in Sent and count only through
\* `$answered`; the Sent fallback reads only other tools' replies, and only when the
\* Message-ID they reference is carried by one mail.
FallbackAnswers == othered \ reused
Answered(m) == "answered" \in kw[m] \/ m \in FallbackAnswers
Open(m) == ~Answered(m) /\ "handled" \notin kw[m]
Deliverable(s) == {m \in Watched(s) : "seen" \notin kw[m] /\ Open(m)}
Remindable(s) == {m \in Watched(s) : "seen" \in kw[m] /\ Open(m)}
Known(s) == printed[s] \cup reminded[s]

TypeOK ==
    /\ arrived \subseteq Mails
    /\ thread \in [Mails -> Threads]
    /\ kw \in [Mails -> SUBSET Keywords]
    /\ replied \subseteq Mails /\ othered \subseteq Mails /\ reused \subseteq Mails
    /\ pc \in [Sessions -> {"idle", "locked", "got", "lost", "claimed"}]
    /\ snap \in [Sessions -> SUBSET Mails]
    /\ lock \in [Hosts -> Sessions \cup {None}]
    /\ printed \in [Sessions -> SUBSET Mails]
    /\ reminded \in [Sessions -> SUBSET Mails]
    /\ receipts \in [Mails -> Nat]
    /\ crashes \in 0..MaxCrashes

Init ==
    /\ arrived = {}
    /\ thread = InitThread
    /\ kw = [m \in Mails |-> {}]
    /\ replied = {} /\ othered = {} /\ reused = {}
    /\ pc = [s \in Sessions |-> "idle"]
    /\ snap = [s \in Sessions |-> {}]
    /\ lock = [h \in Hosts |-> None]
    /\ printed = [s \in Sessions |-> {}]
    /\ reminded = [s \in Sessions |-> {}]
    /\ receipts = [m \in Mails |-> 0]
    /\ crashes = 0

Arrive(m) ==
    /\ m \notin arrived
    /\ arrived' = arrived \cup {m}
    /\ UNCHANGED <<thread, kw, replied, othered, reused, receipts, session>>

Merge(from, into) ==
    /\ from # into
    /\ \E m \in arrived : thread[m] = from
    /\ thread' = [m \in Mails |-> IF thread[m] = from THEN into ELSE thread[m]]
    /\ UNCHANGED <<arrived, kw, replied, othered, reused, receipts, session>>

OtherToolReply(m) ==
    /\ m \in arrived
    /\ othered' = othered \cup {m}
    /\ UNCHANGED <<arrived, thread, kw, replied, reused, receipts, session>>

\* A sender gives another mail of m's thread m's Message-ID.
Reuse(m) ==
    /\ m \in arrived
    /\ \E n \in arrived : n # m /\ thread[n] = thread[m]
    /\ reused' = reused \cup {m}
    /\ UNCHANGED <<arrived, thread, kw, replied, othered, receipts, session>>

\* Every check prints the reminders this session has not shown yet, and takes the host's claim lock.
Lock(s) ==
    /\ pc[s] = "idle"
    /\ lock[HostOf[s]] = None
    /\ lock' = [lock EXCEPT ![HostOf[s]] = s]
    /\ reminded' = [reminded EXCEPT ![s] = @ \cup Remindable(s)]
    /\ pc' = [pc EXCEPT ![s] = "locked"]
    /\ UNCHANGED <<mail, snap, printed, crashes>>

\* Also marks `$answered` where the Sent fallback answers the mail.
Get(s) ==
    /\ pc[s] = "locked"
    /\ snap' = [snap EXCEPT ![s] = Deliverable(s)]
    /\ kw' = [m \in Mails |-> IF m \in FallbackAnswers \cap Watched(s) THEN kw[m] \cup {"answered"} ELSE kw[m]]
    /\ pc' = [pc EXCEPT ![s] = "got"]
    /\ UNCHANGED <<arrived, thread, replied, othered, reused, receipts, lock, printed, reminded, crashes>>

Marked(s) == [m \in Mails |-> IF m \in snap[s] THEN kw[m] \cup {"seen", "mdnsent"} ELSE kw[m]]

\* Marks what it read and releases the lock: another host's claim in between goes unnoticed.
Claim(s) ==
    /\ pc[s] = "got"
    /\ kw' = Marked(s)
    /\ lock' = [lock EXCEPT ![HostOf[s]] = None]
    /\ pc' = [pc EXCEPT ![s] = "claimed"]
    /\ UNCHANGED <<arrived, thread, replied, othered, reused, receipts, snap, printed, reminded, crashes>>

\* The mark lands but its reply is lost; the lock is still held.
ClaimLost(s) ==
    /\ pc[s] = "got"
    /\ kw' = Marked(s)
    /\ pc' = [pc EXCEPT ![s] = "lost"]
    /\ UNCHANGED <<arrived, thread, replied, othered, reused, receipts, snap, lock, printed, reminded, crashes>>

\* Still under the lock, the session reads the mail again: what now carries `$seen` it takes as claimed.
Reget(s) ==
    /\ pc[s] = "lost"
    /\ snap' = [snap EXCEPT ![s] = {m \in @ : "seen" \in kw[m]}]
    /\ lock' = [lock EXCEPT ![HostOf[s]] = None]
    /\ pc' = [pc EXCEPT ![s] = "claimed"]
    /\ UNCHANGED <<mail, printed, reminded, crashes>>

\* The read fails too: the session gives up on this check, and the mail is left to reminders.
RegetFailed(s) ==
    /\ pc[s] = "lost"
    /\ snap' = [snap EXCEPT ![s] = {}]
    /\ lock' = [lock EXCEPT ![HostOf[s]] = None]
    /\ pc' = [pc EXCEPT ![s] = "idle"]
    /\ UNCHANGED <<mail, printed, reminded, crashes>>

\* Prints what it claimed, then sends the receipts its snapshot said were unsent.
Deliver(s) ==
    /\ pc[s] = "claimed"
    /\ printed' = [printed EXCEPT ![s] = @ \cup snap[s]]
    /\ receipts' = [m \in Mails |-> IF m \in snap[s] THEN receipts[m] + 1 ELSE receipts[m]]
    /\ snap' = [snap EXCEPT ![s] = {}]
    /\ pc' = [pc EXCEPT ![s] = "idle"]
    /\ UNCHANGED <<arrived, thread, kw, replied, othered, reused, lock, reminded, crashes>>

Crash(s) ==
    /\ pc[s] # "idle"
    /\ crashes < MaxCrashes
    /\ crashes' = crashes + 1
    /\ pc' = [pc EXCEPT ![s] = "idle"]
    /\ snap' = [snap EXCEPT ![s] = {}]
    /\ lock' = IF lock[HostOf[s]] = s THEN [lock EXCEPT ![HostOf[s]] = None] ELSE lock
    /\ UNCHANGED <<mail, printed, reminded>>

\* `send --reply m`: only m can become answered, and only once the submission
\* reached every recipient.
Reply(s, m) ==
    /\ m \in Known(s)
    /\ \E outcome \in Outcomes :
        /\ kw' = IF outcome = "delivered" THEN [kw EXCEPT ![m] = @ \cup {"answered"}] ELSE kw
        /\ replied' = replied \cup {m}
    /\ UNCHANGED <<arrived, thread, othered, reused, receipts, session>>

Done(s, m) ==
    /\ m \in Known(s)
    /\ kw' = [kw EXCEPT ![m] = @ \cup {"handled"}]
    /\ UNCHANGED <<arrived, thread, replied, othered, reused, receipts, session>>

Wait(s) == Lock(s) \/ Get(s) \/ Claim(s) \/ ClaimLost(s) \/ Reget(s) \/ RegetFailed(s) \/ Deliver(s)

Next ==
    \/ \E m \in Mails : Arrive(m) \/ OtherToolReply(m) \/ Reuse(m)
    \/ \E a, b \in Threads : Merge(a, b)
    \/ \E s \in Sessions : Wait(s) \/ Crash(s)
    \/ \E s \in Sessions, m \in Mails : Reply(s, m) \/ Done(s, m)

\* Strong fairness: a session waiting on the host lock is eventually granted it,
\* which flock does not promise. With weak fairness one busy session can
\* starve the others' claims.
Spec == Init /\ [][Next]_vars /\ \A s \in Sessions : SF_vars(Wait(s))

----------------------------------------------------------------------------
\* Only a reply that references the mail answers it.
AnsweredOnlyByReply == \A m \in Mails : "answered" \in kw[m] => m \in replied \cup othered

OncePerHost ==
    \A m \in Mails, s, t \in Sessions :
        (s # t /\ HostOf[s] = HostOf[t]) => ~(m \in printed[s] /\ m \in printed[t])

Once == \A m \in Mails, s, t \in Sessions : s # t => ~(m \in printed[s] /\ m \in printed[t])

OneReceipt == \A m \in Mails : receipts[m] <= 1

\* An open mail some session watches is eventually shown to a session or closed.
NoLoss ==
    \A m \in Mails :
        (m \in arrived /\ Open(m) /\ \E s \in Sessions : m \in Watched(s))
            ~> ((\E s \in Sessions : m \in Known(s)) \/ ~Open(m))
=============================================================================
