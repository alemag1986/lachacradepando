#!/usr/bin/env python3
"""Provision only the explicitly approved La Chacra Droplet; never log secrets."""
import json
from pathlib import Path
import subprocess
import urllib.request
from integration_env import ROOT, read_env, request_json

NAME = "lachacradepando"
REGION = "nyc3"
SIZE = "s-1vcpu-2gb"

def main():
    token = read_env().get("DIGITALOCEAN_API_TOKEN")
    if not token:
        raise RuntimeError("DIGITALOCEAN_API_TOKEN is missing")
    def api(path, payload=None):
        return request_json("https://api.digitalocean.com/v2/" + path, payload, bearer=token)
    private = ROOT / ".deploy"
    private.mkdir(mode=0o700, exist_ok=True)
    private.chmod(0o700)
    state = private / "digitalocean.json"
    if state.exists():
        print("Provisioning state already exists; inspect it before creating anything else.")
        return
    if any(d["name"] == NAME for d in api("droplets?per_page=200")["droplets"]):
        raise RuntimeError("A matching Droplet already exists; refusing duplicate provisioning")
    size = next(s for s in api("sizes?per_page=200")["sizes"] if s["slug"] == SIZE)
    if size["price_monthly"] != 12 or not size["available"] or REGION not in size["regions"]:
        raise RuntimeError("Approved $12 plan is unavailable; no paid resource created")
    key = private / "id_ed25519"
    if not key.exists():
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", NAME + "-deploy", "-f", str(key)], check=True)
    public = Path(str(key) + ".pub").read_text().strip()
    existing = next((k for k in api("account/keys?per_page=200")["ssh_keys"] if k["public_key"].split()[:2] == public.split()[:2]), None)
    ssh_key = existing or api("account/keys", {"name": NAME + "-deploy", "public_key": public})["ssh_key"]
    with urllib.request.urlopen("https://api.ipify.org", timeout=20) as response:
        operator_ip = response.read(100).decode().strip()
    import ipaddress
    address = ipaddress.ip_address(operator_ip)
    cidr = str(address) + ("/32" if address.version == 4 else "/128")
    # Firewall is installed before provisioning; attach it as soon as the Droplet ID exists.
    firewall = api("firewalls", {
        "name": NAME + "-web",
        "inbound_rules": [
            {"protocol": "tcp", "ports": "22", "sources": {"addresses": [cidr]}},
            {"protocol": "tcp", "ports": "80", "sources": {"addresses": ["0.0.0.0/0", "::/0"]}},
            {"protocol": "tcp", "ports": "443", "sources": {"addresses": ["0.0.0.0/0", "::/0"]}},
        ],
        "outbound_rules": [{"protocol": p, "ports": "all", "destinations": {"addresses": ["0.0.0.0/0", "::/0"]}} for p in ("tcp", "udp")],
    })["firewall"]
    bootstrap = """#cloud-config
package_update: true
packages:
  - docker.io
  - docker-compose-v2
  - sqlite3
  - python3
  - curl
  - ca-certificates
ssh_pwauth: false
disable_root: false
runcmd:
  - [systemctl, enable, --now, docker]
  - [sh, -c, 'fallocate -l 4G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile && echo "/swapfile none swap sw 0 0" >> /etc/fstab']
  - [mkdir, -p, /opt/lachacradepando]
"""
    # Never automatically retry this paid POST after an ambiguous network failure.
    droplet = api("droplets", {"name": NAME, "region": REGION, "size": SIZE,
        "image": "ubuntu-24-04-x64", "ssh_keys": [ssh_key["id"]], "backups": False,
        "ipv6": True, "monitoring": False, "user_data": bootstrap})["droplet"]
    result = {"droplet_id": droplet["id"], "name": NAME, "region": REGION,
        "size": SIZE, "monthly_price": 12, "firewall_id": firewall["id"],
        "ssh_key_id": ssh_key["id"], "operator_cidr": cidr}
    state.write_text(json.dumps(result, indent=2) + "\n")
    state.chmod(0o600)
    api("firewalls/" + firewall["id"] + "/droplets", {"droplet_ids": [droplet["id"]]})
    print("Created approved $12/month Droplet:", droplet["id"])
    print("Firewall attached; SSH restricted to this connection's public IP.")

if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, StopIteration) as error:
        raise SystemExit("Provisioning stopped: " + str(error))
