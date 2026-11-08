---
title: Mobile app
sidebar_position: 23
description: Set up the Nidoo Android app by scanning a QR code from the hub, and add remote access through the encryption gateway.
---

The hub draws a QR code that sets up the Nidoo Android app in one scan. The app reads the hub address and the broker login from it, so you type nothing on the phone.

## Where the code is

- At the end of the setup wizard, in the **Connect your phone** block of the Done step.
- Later, in **Settings → Mobile app**.

The code is blurred. Press **Show code** to reveal it and **Hide** to blur it again.

## Scan it

Install the Nidoo app, open it, tap **Scan setup code** and point the camera at the screen. The app fills in the hub and the broker for you.

By default the code carries the hub's address on your home network, such as `http://192.0.2.10:8081`, so the phone works at home only. The list row in Settings reads **Home network only**.

## Remote access

To use the app away from home, the code must also carry the address of your encryption gateway. In **Remote access**, fill in:

- **Gateway address**: the gateway's https address, for example `https://house.example.ts.net`. It must start with `https://`.
- **Gateway key**: the gateway's 64-character hexadecimal key.

Press **Save**. The hub does not restart. Once both fields are set, the code is redrawn with the gateway and the row reads **At home and away**. Scan the new code with the phone. The fields are also described in the [Settings reference](../reference/configuration.md#mobile).

## Keep the code private

The code holds your passwords: the broker login and the gateway key. Show it only to people you trust, never share a screenshot of it, and blur it again when you are done. The hub never writes it to its logs.
