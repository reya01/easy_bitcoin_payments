# Start here, advanced: any VPS, from the command line

For people who are comfortable with SSH and a terminal and want to put EBP on a
server that is **not** DigitalOcean's paste-in-a-box setup (Hetzner, Vultr, AWS,
a server in a rack at home, and so on). **If you are new to this, use
[START_HERE.md](START_HERE.md) instead.** It needs no terminal.

> **Status.** The steps below are the same scripts the DigitalOcean path uses,
> and CI rehearses them on a clean Ubuntu 24.04 machine. **We have not run them
> on other hosting companies.** Where something is untested it says so. Play money
> (signet) is the default; real money is experimental
> ([START_HERE.md](START_HERE.md#real-money-experimental)).

What you are trusting on any host is in [OTHER_HOSTS.md](OTHER_HOSTS.md): the
host company can see the server's disk, the code you run, and yourself.

## What you need

- A server you can reach with SSH. **x86-64 (amd64) Ubuntu 24.04** is the tested
  setup. Other Linux with Docker and Docker Compose v2 should work but is untested.
  **ARM servers:** the signed image is x86-64 only for now (ARM is on the
  [roadmap](ROADMAP.md)), so on ARM you build from source (section 5, option A, and
  `update.sh --from-source`).
- **2 GB of memory or more.** The first build is the heavy part; on a small server
  add swap (section 2).
- **Ports 80 and 443 reachable from the internet**, plus 22 for SSH. Port 8080,
  where EBP itself listens, is kept on the server's own address
  (`127.0.0.1`) and is not exposed.
- Optional: a domain name (for example `pay.myshop.com`). Without one you get a
  free `<your-ip>.sslip.io` name with a real certificate.
- A password manager. You will save a setup code, an admin password and 12 recovery words.

## 1. Look after the server first

Log in as root once, then:

```
adduser ebp                      # a normal user; give it a long password or none and use SSH keys
usermod -aG sudo ebp
rsync -a ~/.ssh /home/ebp/ && chown -R ebp:ebp /home/ebp/.ssh   # copy your SSH key across
```

Open a **second** terminal and check `ssh ebp@your-server` works **before** you
close the first. Then, as `ebp`:

```
sudo sed -i 's/^#\?PasswordAuthentication .*/PasswordAuthentication no/' /etc/ssh/sshd_config
sudo sed -i 's/^#\?PermitRootLogin .*/PermitRootLogin no/' /etc/ssh/sshd_config
sudo systemctl reload ssh
sudo ufw allow OpenSSH && sudo ufw allow 80/tcp && sudo ufw allow 443/tcp && sudo ufw --force enable
sudo apt-get update && sudo apt-get install -y unattended-upgrades fail2ban
timedatectl | grep -i "synchronized"     # should say yes
```

Why the clock matters: payment notices to your shop are signed with the time and
refused if the two clocks differ by more than five minutes.

(Docker publishes the web server's ports 80 and 443 itself and can bypass ufw
rules for those two; that is fine, they are meant to be open. EBP's own port 8080
is bound to the local address in `docker-compose.yml`. If you change
`EBP_BIND_HOST`, you expose it: do not.)

## 2. Install Docker, git and (if needed) swap

```
sudo apt-get install -y git curl ca-certificates docker.io docker-compose-v2
sudo systemctl enable --now docker
sudo usermod -aG docker ebp      # then log out and back in; or put sudo in front of docker commands
```

If `free -m` shows less than about 2 GB of memory and no swap, add 2 GB (the
DigitalOcean path does the same):

```
sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile
sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
```

## 3. Own domain (optional)

**At your domain company**, add an **A record** for the name (say `pay`) pointing
at the server's IP, then wait a few minutes. `dig +short pay.myshop.com` should
print the IP. If your company offers a proxy ("orange cloud" and similar), turn it
**off** ("DNS only"): the padlock certificate is issued directly to the server.

## 4. Get the code, at a release

Pick a release from the [Releases page](https://github.com/reya01/easy_bitcoin_payments/releases)
(these steps say `VERSION`; an example is `0.3.3-rc6`). Release candidates are
marked pre-release.

```
cd /opt && sudo git clone --depth 1 --branch vVERSION https://github.com/reya01/easy_bitcoin_payments.git ebp
sudo chown -R ebp:ebp /opt/ebp && cd /opt/ebp
```

(The DigitalOcean setup text clones `main`, the newest tested code, which can be
ahead of the newest release. A named release is the safer choice here.)

Read `scripts/start.sh` and `scripts/update.sh` before you run them. They are short.

## 5. First start

**Option A, the rehearsed way: build from source, then move to the signed image.**

```
./scripts/start.sh --https                    # free sslip.io address
# or:  ./scripts/start.sh --domain pay.myshop.com
```

The first run **compiles the server and is quiet for 15 to 30 minutes** on a small
machine. At the end it prints the padlock address and the **SETUP CODE**. Open the
address, enter the code and continue at
[step 4 of START_HERE](START_HERE.md#4-finish-setup-in-your-browser): create the
wallet, **write down the 12 words**, set the admin password and two-step sign-in.

Then switch to the signed prebuilt image, so what runs is what the project's
release workflow built:

```
./scripts/update.sh --check VERSION           # changes nothing, shows the signature result
./scripts/update.sh VERSION                   # asks you to type yes
```

**Option B, skip the long build (not yet rehearsed from an empty server).** The
update script can be the first thing you run: it checks the signature, saves the
signed image's address in `.env`, and starts that image without ever compiling.

```
./scripts/update.sh --check --allow-unknown-running-version VERSION
./scripts/update.sh --allow-unknown-running-version VERSION      # type yes
./scripts/start.sh --https                    # or --domain pay.myshop.com: adds the padlock
```

The first command skips only the "never go backwards" check, because there is no
running server to compare with. If any command errors, use Option A.

Either way, finish setup in the browser at the padlock address. Make a **test
payment** with play money ([START_HERE step 5](START_HERE.md#5-make-a-test-payment)).

## 6. Updates from the admin page (optional)

To update from the **Home** tab instead of the terminal, run once as root:

```
sudo ./scripts/install-update-helper.sh
```

It installs two small systemd units that act only when you press **Update**
(twice, after seeing the signature result). Nothing updates by itself. Remove it
with `sudo ./scripts/install-update-helper.sh --uninstall`.

## 7. Everyday commands

Run these in `/opt/ebp`:

```
docker compose ps                                 # is it running?
docker compose logs --no-color --tail 100 ebp     # the server's log
docker compose logs --no-color --tail 100 caddy   # the padlock and web server
docker compose logs --no-color ebp | grep "SETUP CODE"   # the setup code, only until setup is done
./scripts/start.sh                                # start or restart; keeps your https choice
```

Settings live in `/opt/ebp/.env` (the address, the chosen image). Your wallet and
orders live in a Docker volume named `ebp-data` (Docker adds the folder name in
front). **Do not delete the volume** and do not run `docker compose down -v`.

## 8. Backups

Use **Settings → Download a backup** on the admin page, and keep the file and the
12 words somewhere safe and **separate from the server**. The tested restore steps
are in [RECIPE_BACKUP.md](RECIPE_BACKUP.md) and [RESTORE_DRILL.md](RESTORE_DRILL.md).
Copying the volume by hand is possible, but stop the server first
(`docker compose stop ebp`; the wallet program runs inside it) and treat it as unrehearsed.

## 9. Connect your shop

Same as for every host: [WOOCOMMERCE.md](WOOCOMMERCE.md). The server must be
reachable from your shop over https. EBP refuses to send notices to a shop on a
private address.

## If something goes wrong

- **"The padlock address is not answering":** ports 80/443 are closed (check the
  host's own firewall, not just ufw) or the domain does not point at the server
  yet. Fix it and run `./scripts/start.sh` again. Nothing is lost.
- **The build stops or the server freezes:** it ran out of memory. Add swap
  (section 2) and run `./scripts/start.sh` again; Docker reuses what it already built.
- **"permission denied" from docker:** log out and in again after the `usermod`
  step, or put `sudo` in front.
- **A payment or payout looks wrong:** stop and read
  [IF_SOMETHING_LOOKS_WRONG.md](IF_SOMETHING_LOOKS_WRONG.md). Do not try fixes.

## Not covered or not tested

- Hosts other than a clean Ubuntu 24.04 machine. Tell us what you find.
- ARM servers, other Linux families, rootless Docker, Podman.
- The bare binary (`scripts/install.sh`). It is for developers and installs only
  the program, with no wallet, padlock or service files; see
  [UPGRADING.md](UPGRADING.md#advanced).
- Running the server behind your own reverse proxy: possible
  (`EBP_TRUST_PROXY_HEADERS`), not documented here.
- Option B above, from an empty server.
