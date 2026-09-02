<div align="center">

# Dotrift

*All your settings in one folder. One command to put them in place.*

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE.txt)

</div>

Dotrift is for anyone who has set up a computer and thought: *"I know I fixed this last time… where was it?"*

## The problem

Your computer keeps the settings that make it feel like yours — how your text editor looks, what your terminal does, the small preferences you've tuned over years — tucked away in hidden corners of the system. Moving to a new computer means hunting them down and copying them over by hand: slow, fiddly, and easy to get wrong.

Dotrift turns that around. You keep every setting in one ordinary folder, and Dotrift places them where your computer expects to find them.

```mermaid
flowchart LR
    subgraph s1["Without Dotrift"]
        direction LR
        a["Settings hidden<br>in one corner"]
        b["Settings hidden<br>in another"]
        c["Settings hidden<br>in a third place"]
        a -.->|"copy each one<br>by hand"| h["New computer<br>set up slowly,<br>from memory"]
        b -.-> h
        c -.-> h
    end
    subgraph s2["With Dotrift"]
        direction LR
        f["One folder with<br>all your settings"] --> d["Dotrift"] --> k["New computer<br>ready in one step"]
    end
    style s1 fill:#e6f4ea,stroke:#34a853,stroke-width:2px,color:#34a853
    style s2 fill:#fce8e6,stroke:#ea4335,stroke-width:2px,color:#ea4335
```

## What Dotrift does

- **Keeps everything together** — one folder holds all your settings instead of them being scattered across hidden corners
- **One step, every time** — each file lands exactly where your computer expects it
- **Fills in the blanks** — the same setup can have small differences per machine, like work versus personal
- **Asks first** — Dotrift never replaces a file you've changed without checking with you
- **Keeps track** — it remembers everything it placed, so it can tidy up when your setup moves on

> [!TIP]
> Change a setting once in your folder, and every machine that uses it can pick up the change.

## How it works, in one picture

Every file in your folder travels one of three ways onto your computer:

```mermaid
flowchart LR
    f["One folder with<br>all your settings"] --> d["Dotrift"]
    d -->|"linked"| l["Stays connected<br>to your folder"]
    d -->|"copied"| c["A straight duplicate<br>lands in place"]
    d -->|"filled in"| t["Blank spots completed<br>with your answers"]
    l --> h["Exactly where your<br>computer expects it"]
    c --> h
    t --> h
```

Set it up once, and every computer you use feels like *yours*.
