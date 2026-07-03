#!/usr/bin/env python3
from scapy.all import Ether, IP, UDP, Raw, sendp, get_if_hwaddr
import struct
import time

def send_tx(bitmap, port=8000):
    iface_send = "veth1"

    src_mac = get_if_hwaddr(iface_send)

    payload = struct.pack(">H", bitmap) + b"constellation_tx_data"
    
    pkt = (Ether(src=src_mac, dst="ff:ff:ff:ff:ff:ff") / 
           IP(src="1.1.1.1", dst="1.1.1.1") / 
           UDP(sport=12345, dport=port) / 
           Raw(load=payload))
    
    print(f"[*] Sending to {iface_send} (MAC: {src_mac}) | port: {port} | Bitmap: 0x{bitmap:04X}")
    sendp(pkt, iface=iface_send, verbose=False)

if __name__ == "__main__":
    print("--- Constellation Traffic Generator ---")

    send_tx(0x0001)
    time.sleep(1)

    send_tx(0x0003)
    time.sleep(1)

    send_tx(0x8000)
    time.sleep(1)

    send_tx(0x8000, 9000)
    time.sleep(1)