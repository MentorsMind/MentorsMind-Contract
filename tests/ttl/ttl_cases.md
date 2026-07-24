# TTL Case Matrix

| Case | Contract | Key type | Operation | Expected outcome |
| --- | --- | --- | --- | --- |
| Config key persistence | bounty | admin/config | `initialize` then ledger jump | subsequent `post_bounty` succeeds |
| Bounty record renewal | bounty | persistent `DataKey::Bounty(id)` | `post_bounty` + ledger jump | `get_bounty` returns claimed/open state |
| Claim record renewal | bounty | persistent `(Claim, id, learner)` | `claim_bounty` + ledger jump | `get_claim` returns pending/disputed state |
| Counter key renewal | bounty | instance/persistent count key | repeated posts and reads | count remains monotonic |
| Deadline window behavior | bounty | mixed keys | advance timestamp/sequence | refund/dispute guards remain correct |
