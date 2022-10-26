from asyncio import create_subprocess_exec as proc, run, create_task as spawn
from asyncio.subprocess import PIPE
from asyncio import sleep, gather


async def main():
    p = await proc("cargo", "build", "--release", "--bin", "tidyup")
    await p.wait()
    p = await proc("rsync", "target/release/tidyup", "nsl-node1.d1.comp.nus.edu.sg:~")
    await p.wait()

    task = spawn(remote("nsl-node1.d1.comp.nus.edu.sg"))
    await gather(task, interrupt_remote("nsl-node1.d1.comp.nus.edu.sg"))


async def remote(host):
    # await interrupt_remote(host)

    p = await proc("ssh", host, "./tidyup", stdout=PIPE, stderr=PIPE)
    while not p.stdout.at_eof():
        line = await p.stdout.readline()
        print(line.decode())
    _, err_lines = await p.communicate()
    if p.returncode != 0:
        print(f"--- Exit code {p.returncode}")
        print("--- Standard error start")
        print(err_lines.decode())
        print("--- Standard error end")


async def interrupt_remote(host):
    await sleep(10)
    print("Interrupt remote")
    p = await proc("ssh", host, "pkill", "-INT", "tidyup")
    await p.wait()


run(main())
