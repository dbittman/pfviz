# pfviz

Visualize page faults and cache misses to memory-mapped files during a running program.

## Basic Usage

This program has two basic modes, trace and play. In trace mode, the program will run another program and use perf to trace it, capturing page faults and additional events. The user can specify any number of extra events to capture with the -e flag. For example, on machines with this particular perf event, cache misses can be traced like this:

`cargo run --release -- trace -e mem_load_retired.l3_miss:ppu,miss`

The ',miss' is to inform pfviz what kind of event this is.

Note that not all events are supported on all architectures or CPUs. Check perf list for events. For cache miss tracking, the event will need to be a "precise" event.

In play mode, the program will draw a TUI to play back the events visually. Each rendered rectangle is a memory-mapped file. During playback, the program will color in regions within the rectangle to indicate access. There are two bars drawn per file: a visualization of cache-misses (top) and page-faults (bottom).

Playback can be paused, looping can be toggled, and markers in the playback timeline can be set (that act as start and end for looping). On invocation, playback mode and speed can be selected (see --help).

## License

Copyright (c) Daniel Bittman <danielbittman1@gmail.com>

This project is licensed under the MIT license ([LICENSE] or <http://opensource.org/licenses/MIT>)

[LICENSE]: ./LICENSE
