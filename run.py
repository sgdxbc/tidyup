from asyncio import create_subprocess_exec as proc, run, create_task as spawn
from asyncio.subprocess import PIPE
from asyncio import sleep, gather
from os import environ

PROG = "tidyup-v2"
sync_nodes, kill_nodes, wait_nodes = [], [], []

if environ.get("CI"):
    print("[R] * CI mode enabled")
    CI = True
else:
    CI = False

async def main():
    p = await proc(
        "cargo", "run", "--quiet", "--package", "run", "--", "export", stdout=PIPE
    )
    out, _ = await p.communicate()
    assert p.returncode == 0
    for line in out.decode().splitlines():
        if line.startswith("SYNC"):
            sync_nodes.append(line.replace("SYNC", "", 1).strip())
        if line.startswith("RUN_KILL"):
            kill_nodes.append(line.replace("RUN_KILL", "", 1).strip())
        if line.startswith("RUN_WAIT"):
            wait_nodes.append(line.replace("RUN_WAIT", "", 1).strip())
    if CI:
        assert all(node.startswith("127.") for node in sync_nodes + kill_nodes + wait_nodes)
    print("[R] * Exported")

    p = await proc("cargo", "build", "--release", "--quiet", "--bin", PROG)
    await p.wait()
    assert p.returncode == 0
    if not CI:
        ps = [
            await proc("rsync", f"target/release/{PROG}", f"{host}:~")
            for host in sync_nodes
        ]
        await gather(*(p.wait() for p in ps))
        assert all(p.returncode == 0 for p in ps)
    else:
        p = await proc("cp", f"target/release/{PROG}", "/tmp")
        await p.wait()
        assert p.returncode == 0
    print("[R] * Synchronized")

    tasks = [spawn(remote(host, f"[{i}]")) for i, host in enumerate(kill_nodes)]
    wait_tasks = [spawn(remote(host, "[C]")) for host in wait_nodes]

    if not CI:
        await sleep(2)
    else:
        await sleep(10)
    p = await proc("cargo", "run", "--quiet", "--package", "run", "--", "liftoff")
    await p.wait()

    await gather(*wait_tasks)
    await gather(*(interrupt_remote(host) for host in kill_nodes))
    await gather(*tasks)


async def remote(host, tag):
    await interrupt_remote(host)

    if not CI:
        p = await proc("ssh", host, f"./{PROG}", stdout=PIPE, stderr=PIPE)
    else:
        p = await proc(
            f"/tmp/{PROG}",
            stdout=PIPE,
            stderr=PIPE,
            env={"SSH_CONNECTION": f"127.0.0.1 0 {host} 22"},
        )
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
    if not CI:
        p = await proc("ssh", host, "pkill", "-INT", PROG)
    else:
        assert host.startswith("127.")
        p = await proc("pkill", "-INT", PROG)
    await p.wait()


try:
    run(main())
except KeyboardInterrupt:
    print()
    print("Aborting")

    async def clean_up():
        await gather(*[interrupt_remote(host) for host in kill_nodes + wait_nodes])

    run(clean_up())
