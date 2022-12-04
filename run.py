from asyncio import create_subprocess_exec as proc, run, create_task as spawn
from asyncio.subprocess import PIPE
from asyncio import sleep, gather

nodes = [
    "nsl-node1.d1.comp.nus.edu.sg",
    "nsl-node2.d1.comp.nus.edu.sg",
    "nsl-node3.d1.comp.nus.edu.sg",
    #
]
wait_nodes = [
    "nsl-node5.d1.comp.nus.edu.sg",
    #
]


async def main():
    p = await proc("cargo", "build", "--release", "--quiet", "--bin", "tidyup-v2")
    await p.wait()
    assert p.returncode == 0
    p = await proc("rsync", "target/release/tidyup-v2", "nsl-node1.d1.comp.nus.edu.sg:~")
    await p.wait()
    assert p.returncode == 0
    print("[R] * Updated")

    tasks = [spawn(remote(host, f"[{i}]")) for i, host in enumerate(nodes)]
    wait_tasks = [spawn(remote(host, "[C]")) for host in wait_nodes]

    await sleep(2)
    p = await proc("cargo", "run", "--quiet",  "--package", "run")
    await p.wait()

    await gather(*wait_tasks)
    await gather(*[interrupt_remote(host) for host in nodes])
    await gather(*tasks)


async def remote(host, tag):
    await interrupt_remote(host)

    p = await proc("ssh", host, "./tidyup-v2", stdout=PIPE, stderr=PIPE)
    while not p.stdout.at_eof():
        line = await p.stdout.readline()
        if line:
            print(f"{tag} {line.decode()}", end="")
    _, err_lines = await p.communicate()
    if p.returncode != 0:
        print(f"{tag} Exit code {p.returncode}")
        print("--- Standard error")
        print(err_lines.decode(), end="")
        print("--- Standard error end")


async def interrupt_remote(host):
    p = await proc("ssh", host, "pkill", "-INT", "tidyup")
    await p.wait()


try:
    run(main())
except KeyboardInterrupt:
    print()
    print("Aborting")

    async def clean_up():
        await gather(*[interrupt_remote(host) for host in nodes + wait_nodes])

    run(clean_up())
