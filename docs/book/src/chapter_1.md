# Yet another Rust embeeded book?

This is a book sparking from a discussion on the matrix esp-rs channel, where the author was asking for a structured project pattern for async Rust on embedded devices, specifically using the embassy ecosystem AND Espressif's esp-hal as the Hardware Abstraction Layer.

I think that the words from `@ivmarkov` set the stage for this book quite well for what I was looking for:

> After all, folks are likely re-inventing the same thing in their firmware all over again - a bunch of reactive tasks modeled with async tasks scheduled in one or two embassy executors. Same as what you would do with non-async Rust on top of an RTOS, except with async-rust your tasks are async tasks.
>
> Some of these tasks have a higher prio / lower latency constraints (dealing with tricky hw), and some "can wait" and "tolerate higher latency" (perhaps like smoltcp networking code).
>
> And then also some peripheral-but-important topics come, like OTA updates, NVS storage and so on.
>
>Picking one concrete example might help. Back in the days in my initial struggles, I had this "water-meter-with-an-emergency-valve-and-an-lcd-screen" project (now hopelesssly outdated). Perhaps picking one such example for the book and going thru it would be good enough to cover most of the topics.

# Rules of engagement

Somebody said that engineers thrive under constraints? Here are (non negotiable?) "principles" for my project:

* no_std
* no alloc
* no unsafe
* use embassy ecosystem

Hopefully those same self-imposed rules help you in creating lean, performant and safe async Rust firmware for embedded projects.