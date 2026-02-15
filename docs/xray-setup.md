# Xray setup for proxy_agent

Yes, you need **Xray-core** running. The proxy_agent syncs user UUIDs from the control plane and adds/removes them on your Xray VLESS inbound via gRPC. Without Xray, the VPN has no server to push users to.

## 1. Install Xray on Linux

**Option A – install script (recommended)**

```bash
bash -c "$(curl -L https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install
```

- Binary: `/usr/local/bin/xray`
- Config dir: `/usr/local/etc/xray/`
- Config file: `/usr/local/etc/xray/config.json`

**Option B – manual**

1. Download the right build from [Xray-core releases](https://github.com/XTLS/Xray-core/releases) (e.g. `xray-linux-64.zip`).
2. Unzip and put `xray` in your PATH (e.g. `/usr/local/bin/xray`).
3. Create a config file (e.g. `/usr/local/etc/xray/config.json`).

## 2. Enable the gRPC API (HandlerService)

Xray must expose the **HandlerService** gRPC API so proxy_agent can add/remove users.

**Simplified API (Xray v1.8.12+)** – add to your config:

- Use **`0.0.0.0:8080`** for `api.listen` if proxy_agent runs in Docker and Xray on the host; otherwise the container cannot reach the API.

```json
{
  "api": {
    "tag": "api",
    "listen": "0.0.0.0:8080",
    "services": ["HandlerService"]
  },
  "inbounds": [
    {
      "tag": "inbound-vless",
      "listen": "0.0.0.0",
      "port": 443,
      "protocol": "vless",
      "settings": {
        "clients": [],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "tcp"
      },
      "sniffing": {
        "enabled": true,
        "destOverride": ["http", "tls"]
      }
    }
  ],
  "outbounds": [
    {
      "protocol": "freedom",
      "tag": "direct"
    }
  ]
}
```

- **`api.listen`** – this is your **gRPC API URL** (see below).
- **`inbounds[].tag`** – must match what proxy_agent uses (`XRAY_INBOUND_TAG`, default `inbound-vless`). The snippet uses `inbound-vless`.

Adjust `inbounds` (ports, TLS, etc.) to your real VLESS setup; the important part is `api` and the inbound `tag`.

**If you use the "inbound + routing" style** (no `api.listen`, dokodemo-door on 8080 with tag `api` and routing to outbound `api`): do **not** add an outbound with `"tag": "api"` yourself. Xray creates the API outbound automatically; if you add e.g. `"protocol": "blackhole", "tag": "api"`, API traffic will be dropped and proxy_agent will get "transport error". Remove that outbound and keep only `direct` (and any others you need).

## 3. What URL to use for proxy_agent

The gRPC URL is **`http://<api.listen host>:<api.listen port>`**.

From the example above, `"listen": "127.0.0.1:8080"` gives:

| Where proxy_agent runs  | XRAY_GRPC_ADDR                     |
| ----------------------- | ---------------------------------- |
| Same Linux host as Xray | `http://127.0.0.1:8080`            |
| In Docker, Xray on host | `http://host.docker.internal:8080` |

So:

- **Xray and proxy_agent on the same machine**  
  Use `http://127.0.0.1:8080` (or whatever host:port you set in `api.listen`).

- **proxy_agent in Docker, Xray on the host**  
  In `.env` set:
  ```bash
  XRAY_GRPC_ADDR=http://host.docker.internal:8080
  ```
  (docker-compose already adds `host.docker.internal` for Linux.)

Change `8080` if your `api.listen` uses another port.

## 4. Check that the API is reachable

On the host where Xray runs:

```bash
# List gRPC services (requires grpcurl)
grpcurl -plaintext 127.0.0.1:8080 list
```

You should see something like:

```
grpc.reflection.v1alpha.ServerReflection
xray.app.proxyman.command.HandlerService
```

If that works, the URL is correct and proxy_agent can use it (with `http://` and the same host:port from the same or from Docker as above).

**Install grpcurl (optional):**

- Debian/Ubuntu: `sudo apt install grpcurl`
- Or: <https://github.com/fullstorydev/grpcurl#installation>

### Test from Docker (Xray on host, proxy_agent in Docker)

If `grpcurl -plaintext 127.0.0.1:8080 list` works on the host but proxy_agent in Docker gets "transport error", test whether **any** container can reach the host’s gRPC:

```bash
docker run --rm --add-host=host.docker.internal:host-gateway fullstorydev/grpcurl -plaintext host.docker.internal:8080 list
```

- **If this fails:** the host is not accepting connections on 8080 from Docker. Typical causes:
  - **Firewall:** allow TCP 8080 from Docker (e.g. from `172.17.0.0/16` / `172.18.0.0/16`). Example for `iptables`:  
    `sudo iptables -I INPUT -p tcp -s 172.17.0.0/16 --dport 8080 -j ACCEPT`  
    (and same for 172.18.0.0/16 if you use a compose network.)
  - **Xray only on localhost:** if you use the "inbound + routing" style, ensure the API inbound (e.g. dokodemo-door) has `"listen": "0.0.0.0"` so it listens on all interfaces, not just `127.0.0.1`.
- **If this works:** the network path is fine; the problem may be specific to the proxy_agent client (e.g. HTTP/2). You can still run proxy_agent **outside Docker** on the same host as Xray with `XRAY_GRPC_ADDR=http://127.0.0.1:8080` to confirm the rest of the flow.

## 5. Start Xray

**With systemd (after install script):**

```bash
sudo systemctl enable xray
sudo systemctl start xray
sudo systemctl status xray
```

**Manual run:**

```bash
xray run -c /usr/local/etc/xray/config.json
```

Keep this config path and `api.listen` in mind when setting `XRAY_GRPC_ADDR` and optional `XRAY_INBOUND_TAG` for proxy_agent.
