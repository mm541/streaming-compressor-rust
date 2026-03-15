# Assembler Architecture

The `assembler.rs` module converts a scattered directory of files with varying sizes into a **contiguous, sequential stream of fixed-size data blocks** (fragments).

It dynamically spans across file boundaries, tracking offsets, boundaries, and checksums lazily so that everything runs with minimal memory overhead, processing chunks block-by-block.

## 1. Boundary-Spanning Data Flow

This diagram illustrates how files on disk map into a virtual infinitely contiguous byte stream, and how the `assembler.rs` iterator flawlessly slices that stream into fixed `fragment_size` targets (e.g. 1MB).

```mermaid
graph LR
    subgraph Files on Disk
        File1[File 1 <br/> 1.5 MB]
        File2[File 2 <br/> 0.8 MB]
        File3[File 3 <br/> 1.0 MB]
    end

    subgraph Virtual Byte Stream
        Stream[Consecutive Byte Stream <br/> 3.3 MB Total]
    end

    subgraph FragmentStream Output
        Frag1[Fragment 1 <br/> 1.0 MB]
        Frag2[Fragment 2 <br/> 1.0 MB]
        Frag3[Fragment 3 <br/> 1.0 MB]
        Frag4[Fragment 4 <br/> 0.3 MB]
    end

    File1 --> |0.0 - 1.5| Stream
    File2 --> |1.5 - 2.3| Stream
    File3 --> |2.3 - 3.3| Stream

    Stream --> |0.0 - 1.0| Frag1
    Stream --> |1.0 - 2.0| Frag2
    Stream --> |2.0 - 3.0| Frag3
    Stream --> |3.0 - 3.3| Frag4

    classDef file fill:#2a9d8f,stroke:#264653,stroke-width:2px,color:#fff;
    classDef stream fill:#e9c46a,stroke:#e76f51,stroke-width:2px,color:#333;
    classDef frag fill:#e76f51,stroke:#264653,stroke-width:2px,color:#fff;

    class File1,File2,File3 file;
    class Stream stream;
    class Frag1,Frag2,Frag3,Frag4 frag;
```

## 2. Iterator Internal Logic Map

How the `FragmentStream::next()` method determines how many bytes to read, handles reaching the end of individual files concurrently while building a single Fragment buffer.

```mermaid
stateDiagram-v2
    direction TB

    state "FragmentStream::next()" as Start
    state "Initialize empty Vec<u8> buffer" as Init
    state "Loop while space_left > 0" as LoopCheck

    state "Yield Complete Fragment" as Yield

    state "Check if remaining_in_file == 0" as FileCheck
    state "Advance to Next File" as Advance
    state "No more files?" as EOFCheck
    state "Yield Partial Fragment (EOF)" as YieldPartial

    state "Calculate bytes to read: min(space_left, remaining)" as Calc
    state "Read exact bytes into buffer" as Read

    state "Decrement counters" as UpdateCounters
    state "Finalize File Checksum if EOF" as CheckEOF

    Start --> Init
    Init --> LoopCheck

    LoopCheck --> FileCheck : space_left > 0
    LoopCheck --> Yield : Buffer is full

    FileCheck --> Advance : Yes
    FileCheck --> Calc : No

    Advance --> EOFCheck
    EOFCheck --> YieldPartial : Yes
    EOFCheck --> Calc : No, opened successfully

    Calc --> Read
    Read --> UpdateCounters
    UpdateCounters --> CheckEOF
    CheckEOF --> LoopCheck
```
