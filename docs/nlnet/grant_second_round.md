Dear Michiel, NLNet team,

I hope this (unasked for?) email finds you and your team well amid the tsunami of proposals and requests you are tackling right now. Please, bear with me with this one.

Since before submitting this grant request, I've been writing and testing a proof of concept for this project, as a hobby.
 
But only recently I've started to appreciate the potential and profound impact that this project can have on many ways,
here's just three of them I'd like to hightlight:

1. Hobbyists, makers and hackers tooling.
2. Consumer electronics "augmentation".
3. Right to repair.

At least once a day after work, I frequent a so-called "Hackerspace" where I share common interests with other folks about hardware and software.
I've lost count on the times that me and my colleages have struggled to add robust access and/or telemetry to a new project: SDCards on RaspberryPi
boards get corrupted, too much power consumption means the device has to be tethered (thus robots have limited mobility), custom protocols and servers
mean that time that was meant to be spent on the main project sinks elsewhere, etc... So, having a ROBUST and dependable device that one can SSH into
and customise it to be a UART log capture (even connected to a logic analyser!) would be a game changer in those scenarios. 

For (Linux) embedded systems, having such a "supervisor SSH board" could mean the difference between missing the occasional kernel panic dumped on the 
serial console (and reporting it) versus "we're just rebooting the router because it hangs sometimes" and carry on with limping, broken, potentially vulnerable software.

I have a [keen interest in reverse engineering][stm8_reverse_engineering] and understanding how things work. I believe that adding out of band access to (proprietary) 
consumer electronics should be easier and can unlock many current blackboxes present in our lives. Adding the esp-ssh-rs "stamp" PCB on a domestic treadmill, like the
one I linked above and presented in r2con, means that hackers around the world could band together and decipher complex devices and interfaces as long as a SSH client is available.

Understanding the two exhibits above, one can see how big the appeal is for the Right to Repair situation worldwide: placing a relatively low power PCB
on any device can help greatly in repairing efforts. Gathering data from John Deere tractors to your everyday dishwasher finite state machine **over a SSH connection that you own**
means that you can, with skill, regain control of your devices and keep IoT enshittification at bay... and relay those learnings to the world.

While working on this project I've been indirectly preparing myself for your software "breakdown of the main tasks" ([which I personally find challenging][washing_machine_software_estimation]).

I've already spent a fair amount of hours and occasional embedded consulting fees (gladly!) and reached an early prototype that partially matches two points from my original proposal:

- Wireless AP that bridges UART with SSH **AND** written in memory-safe, `no_alloc` and `no_std` Rust.

In my last few months journey, I encountered many challenges and strange HAL (Hardware Abstraction Layer) limitations while debugging issues that didn't occur to me they'd pose a serious challenge before
(see [washing_machine_software_estimation][washing_machine_software_estimation] for a good real world analogy).

So with this knowledge, here goes a more precise breakdown of tasks (as of March 2025):

1. To have a WiFi AP/STA device that a user can SSH into and securely manage any other device with an UART.
	1. Prototype costed me (out of pocket) around 500€, needs more refinement, so probably should cost no more than 900€ at this point.

2. The device should be relatively effortless to deploy and provision with the required secret key material.
	1. Challenging as there are as many ways to onboard devices as opinions about it. But applyting simplicity and involving third parties in testing, I'd budget this at an additional 800€.

3. Written in embedded Rust (ideally no_std and no alloc to reduce memory fragmentation and allow long runtimes without memory issues).
	1. Many of the unsafe issues have been circumvented, but way more work is needed to make this robust.
	2. Espressif UART-DMA serial driver vs Interrupt driver: Implementing the most suitable solution that does not overrun or glitch the UART (has happened), ~700€
	3. Porting to as many Espressif targets as possible, taking care of memory requirements and setting up HIL (Hardware In the Loop) testing jigs: ~800€
	4. [Sans-IO refactor][sans-io]: The current prototype needs a cleaner decoupling of finite state machines and IO, but a careful focus on performance, ~1500€
	5. Run SSH audit with specialised tools such as SSHambles by HDmoore et al: 300€


This breakdown doesn't include designing and producing the PCB, only developing and testing the software on its target hardware and fixing the bugs and challenges found. 
It doesn't reach the asked-for 8000€ either from the original submission since I'm fully aware that the rest of the cost will be spent in-between those tasks (unknown unknowns). **Hardware IS hard. Software on constrained hardware is ALSO hard(er?)**.

Last but not least, I'm very open to adjusting my proposed grant amount to another one that NLnet considers best suited for this project's "urgency, relevance and value for money". I believe in this project and I'm commited to push it into existence.
For now I'm keeping this project under wraps (private repository) until it matures a bit more, in the hopes of at least economically breaking even with it and learning more about the realities of the manufacturing business side of it.

Full disclosure, my current model is "closed-then-open" depending on gauged public interest (for lack of a better name), [as I proposed a few months ago to NanoKVM authors (seems to have worked well for them so far?)][closed-then-open].

To be frank, I don't particulary like my own proposed "closed-then-source" model to Sipeed's NanoKVM, but I think it can play well with the realities of OSS developers and in particular hardware open source developers. I've never managed to
monetise hardware myself independently, so my highest hope here is NLNet enabling me to change that statement and going forward?

And to be consistent with my remarks, if and when I get accepted in this grant, I'll publish all current source code in a permissive OSS license immediately, from day 1.

Best regards,

Roman Valls Guimera (a.k.a brainstorm)


[washing_machine_software_estimation]: https://www.cosive.com/blog/my-washing-machine-refreshed-my-thinking-on-software-effort-estimation
[stm8_reverse_engineering]: https://github.com/brainstorm/treadmill-re
[sans-io]: https://www.firezone.dev/blog/sans-io
[closed-then-open]: https://github.com/sipeed/NanoKVM/issues/1#issuecomment-2246900903
[sshamble]: https://www.runzero.com/sshamble/