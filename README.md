## A Rust codebase for easily developing/comparing distributed protocols/applications for research propose

Goals of this project:
*   Define a Rust-friendly abstraction for network actors.
Through the implementation of minimal `State` and `SharedState` traits, underlying runtime can learn about:
    *   How to deploy different part of single heavyweight actor into different threads, maximize concurrent processing
    *   How to share one thread with multiple lightweight actors, cooperatively schedule among them
    *   How to fully customize the progress of actors in a simulated environment for test propose

    The abstraction is in synchronous poll mode, avoid unnecessarily messing up with Rust's mutability and lifetime semantic.
*   Provide high performance common *programs* for various proposes.
For example, `program/replica` assigns available threads properly for a replica actor, and `program/bench_client` allows large amount of close-loop clients concurrently working on the same machine.
*   Provide implementation of several well-known distributed protocols (along with an unreplicated simple RPC baseline) for comparison.
*   Provide a handful evaluation solution. You only need to look at one terminal tab for all time ^_^

Other design choices:
*   Although aim for high performance, we don't squeeze performance at the cost of losing simplicity.
All dependent crates are introduced as extensions of standard library. For the ones presenting in root package:
    *   `bincode` for (de)serialize in one line
    *   `crossbeam-channel` for multiple-consumer channel
    *   `nix` for complementary OS routines like `epoll`
    *   `secp256k1` for digital signature

    As a negative example, `rustc-hash` is not used even when secure is not concerned.
    We focus on algorithm and reduce factors that may affect performance as much as possible.
*   Workspace and sub-packages are set up based on previous experiments and for speedup compilation. 
`src/message` serves for two proposes:
    *   Separate data structures that derives `Serialize` and `Deserialize` from root package. 
    It takes long to generate those implementations. 
    And in most of the time we tweak protocols without modifying message spec, where this can help to prevent regeneration.
    *   Avoid `src/run` depending on root package

    `src/run` is a script-like package for rapidly evaluate same implementation (i.e. identical root and `src/message`) with different parameters.
    It is kept minimal and fast to recompile, so we can directly modify its code for all the time instead of defining and editing yet another text-based configuration file.
*   Avoid type parameter, especially on actor types.
*   Send configuration with TCP instead of specifying numerous command line arguments and configuration files. 
It really clears a lot of conventions and boilerplate ^_^

Example benchmark results will be added.

Have fun developing!
