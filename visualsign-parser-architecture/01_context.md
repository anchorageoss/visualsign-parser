# Context Diagram (C4 Level 1)

This diagram illustrates the high-level interaction between external systems and the VisualSign Parser.

```mermaid
C4Context
    title System Context: VisualSign Parser

    Person(user, "User / Wallet", "Initiates a transaction and requests verification.")
    System(custodian, "Custodian / Signing Service", "Holds keys (e.g. Turnkey). Orchestrates signing flow.")
    
    System(visualsign, "VisualSign Parser", "Decodes raw transaction payloads into semantic intent.")
    
    System_Ext(policy, "Policy Engine", "Automated rules verifying intent (e.g. 'Allow Uniswap only').")

    Rel(user, custodian, "1. Request Transaction")
    Rel(custodian, visualsign, "2. Parse(unsigned_payload, metadata)")
    Rel(visualsign, custodian, "3. Return VisualIntent")
    Rel(custodian, policy, "4. Verify(VisualIntent)")
    Rel(policy, custodian, "5. Approve / Reject")
    Rel(custodian, user, "6. Return Result / Signature")

    UpdateElementStyle(user, $bgColor="lightgreen", $borderColor="#333")
    UpdateElementStyle(custodian, $bgColor="lightgreen", $borderColor="#333")
    UpdateElementStyle(visualsign, $bgColor="#99ff99", $borderColor="#333")
    UpdateElementStyle(policy, $bgColor="#ccc", $borderColor="#333")
```

## Legend

-   **User/Wallet**: The end-user or dApp proposing a transaction.
-   **Custodian**: The system integrating VisualSign (e.g., an exchange, MPC wallet provider).
-   **VisualSign Parser**: The system documented here.
-   **Policy Engine**: A downstream consumer of the parsed intent to enforce security rules.

