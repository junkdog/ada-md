# Diagrams

A sequence diagram, drawn:

```mermaid
sequenceDiagram
    participant Client
    participant Gateway
    participant Ledger as Ledger<br/>service
    autonumber
    Client ->> Gateway: POST /v2/transfers (amount, currency, idempotency key)
    Gateway ->> Gateway: Validate the request body against the transfer schema
    loop until the ledger answers
        Gateway ->> Ledger: Reserve funds for the transfer and hold them for fifteen minutes
        Ledger -->> Gateway: Reservation(id, expiresAt)
    end
    Note over Gateway,Ledger: Holds expire on their own
    alt reserved
        Gateway -->> Client: 202 Accepted
    else no funds
        Gateway --x Client: 409 Conflict
    end
```

Inside a quote:

> ```mermaid
> sequenceDiagram
>     A->>B: hello
>     B-->>A: hi
> ```

A flowchart, with a subgraph:

```mermaid
flowchart LR
    subgraph parse[Parse]
        A[read file] --> B{markdown?}
    end
    B -->|yes| C[pulldown-cmark]
    B -->|no| D[syntect]
    C --> E[layout]
    D --> E
    E -.-> F((paint))
```

A state diagram:

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Loading: open
    Loading --> Ready: parsed
    Loading --> Failed: error
    Failed --> Idle: retry
    Ready --> [*]
```

A kind ada doesn't draw stays source:

```mermaid
pie title Frame time
    "Layout" : 40
    "Paint" : 60
```

So does one that doesn't parse:

```mermaid
sequenceDiagram
    this is not a statement
```
