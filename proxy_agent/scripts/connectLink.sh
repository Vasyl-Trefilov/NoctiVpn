#!/bin/bash

UUID="251565a6-e155-473b-b4c4-dbe022a93c4f"
IP="176.199.209.116"
PUBKEY="Vk_e5sQQSWgy4dumRpAvupAmEnWZDHWeRy4galA2YBs"

echo "vless://$UUID@$IP:443?encryption=none&security=reality&sni=www.cloudflare.com&fp=chrome&pbk=$PUBKEY&type=tcp&flow=xtls-rprx-vision#REALITY-Server"