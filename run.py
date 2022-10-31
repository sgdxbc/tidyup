from asyncio import create_subprocess_exec as proc, run, create_task as spawn
from asyncio.subprocess import PIPE
from asyncio import sleep, gather

replicas = [
    "nsl-node1.d1.comp.nus.edu.sg",
    "nsl-node2.d1.comp.nus.edu.sg",
    "nsl-node3.d1.comp.nus.edu.sg",
    #
]
clients = [
    "nsl-node4.d1.comp.nus.edu.sg",
    #
]


async def main():
    p = await proc("cargo", "build", "--release", "--bin", "tidyup")
    await p.wait()
    assert p.returncode == 0
    p = await proc("rsync", "target/release/tidyup", "nsl-node1.d1.comp.nus.edu.sg:~")
    await p.wait()
    assert p.returncode == 0

    replica_tasks = [spawn(remote(host, f"[{i}]")) for i, host in enumerate(replicas)]
    client_tasks = [spawn(remote(host, "[C]")) for host in clients]

    await sleep(2)
    p = await proc("./target/release/tidyup", "command")
    await p.wait()

    await gather(*client_tasks)
    await gather(*[interrupt_remote(host) for host in replicas])
    await gather(*replica_tasks)


async def remote(host, tag):
    await interrupt_remote(host)

    p = await proc("ssh", host, "./tidyup", stdout=PIPE, stderr=PIPE)
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
        await gather(*[interrupt_remote(host) for host in replicas + clients])

    run(clean_up())
