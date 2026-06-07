# Requirements

**Draft chapter, here are some raw discussion notes (from several third party authors I'll quote if they want to be quoted!) for now as we decide what's "best" (simpler?) for each usecase?**

**This chapter should go through how to safely share data between async tasks without compromising latency, performance and "ergonomics" while implementing whichever approach**

> Are you looking specifically for an "async cancellation" code or are you just trying to figure out how to share stuff amongst tasks "without mutexes"? Where "without mutexes" (that is, critical-section-based mutexes) would only be possible if all your tasks that need access to the data are confined to a single thread-mode embassy executor?

To begin with, I'd be happy with config sharing between async tasks, no cancellation needed nor any other task manipulation logic needed other than reload `uart_task` config(s) when needed (see [chapter 2.1](chapter_2_1.md)).

> - You DO still need a criticalsectionmutex (or similar) when dealing with the `InterruptExecutor` UART task, as that task can preempt the normal embassy tasks.
> - Unless the data you're sharing access to is large (and therefore you don't want to copy it), its often simpler to have [message passing channels][embassy_channels] that pass updates between tasks rather than any kind of shared state.

<!--
Stuff like interior mutability Rc vs Arc what 'static really means
what a blocking_mutex::Mutex<NoopRawMutex, ...> has on top of and comparing to a regular RefCell (hint: nothing; it is just a safer pattern to express the same except that you can't get it wrong - as with RefCell - and erroneously keep a RefCell guard (RefMut) across await points)
-->

> When you want to share mutable stuff across multiple tasks in the same, "thread-mode" embassy-executor (let's put interrupt executors aside).

Let's not put them aside since I'm using them in SSH Stamp, see points made above.

> 1. For blocking stuff: `embassy_sync::blocking_mutex::Mutex<NoopRawMutex, RefCell<T>>` gives you interior mutability, but since you use NoopRawMutex, it is not a "real" mutex so to say, and you don't pay the "mutex" overhead. You share this mutex either with make_static or with Rc (the latter I have to check, but relatively sure it should work; why not?)
>
> 2. For async stuff: `embassy_sync::Mutex<NoopRawMutex, T>` - also gives you interior mutability without a real mutex (so no overhead). Again, either make_static, or Rc should work prior to locking the async "mutex".
> 
> By "real" mutex above I mean like a critical section raw mutex (`CriticalSectionRawMutex`, as opposed to `NoopRawMutex`), which you should not need, as long as all this stuff is shared only within a thread-mode executor, created from within the main async task and spawned using the spawner you have as an arg in that main task.

> Also an alternative to embassy_sync::blocking_mutex::Mutex<NoopRawMutex, T> is just a RefCell.

> There are other small complications, like blocking_mutex::Mutex does not have a RefCell inside by defalt (as the async variant has) so you have to put it yourself (or use Cell if you want to store inside small stuff which is Copy), but this is a detail.

[embassy_channels]: https://docs.embassy.dev/embassy-sync/git/default/channel/struct.Channel.html