# Business Authorization Adapter

The acceptance adapter resolves an `EnterpriseUser` into six allowlists:
legal entity, warehouse, customer, supplier, brand and business unit. Every
request intersects user-supplied filters with those allowlists; it never unions
or widens them. Unknown users fail closed, and an unauthorized exact id is
indistinguishable from a missing id.

The Gateway first consumes a one-turn Delegation bound to the user, active
binding, Agent, turn, source Buzz event/channel, Trace and read scope. The
Business Read API independently verifies that context before work and again
before returning. Revocation at either check discards the result.

The bundled UUIDs and allowlists are desensitized acceptance principals. A
production cutover must replace `acceptance_scope` with the authoritative
business permission service and preserve the same intersection/fail-closed
contract. Permission results may be cached only by user, effective scope hash
and source snapshot; Delegation and session tokens may not be cached.
