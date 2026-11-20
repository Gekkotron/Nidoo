---
title: Wake up to music
sidebar_position: 20.6
description: Set a music alarm in the Wake-up page. Pinson plays a playlist on the Nidoo speakers at a fixed time on chosen weekdays, with a fade-in.
---

The **Wake-up** page sets music alarms. At the time you choose, on the weekdays
you pick, Pinson (a [media player](media-players.md)) starts a playlist and
raises the volume slowly, on the speakers you tick.

Each alarm is a normal [flow](flows.md) that the page writes for you. It runs
like any other flow, shows up in the flows list with a « Wake-up » label, and
is rewritten every time you save it from the page.

## Turn it on

Wake-up is off by default. Open **Settings → Home → Wake-up** and turn on
**Turn on Wake-up**. A **Wake-up** entry then appears in the Home menu. The
same page shows whether a Pinson player is connected and how many playlists and
outputs it lists.

The page needs a media player that can play a playlist (Pinson). Without one it
says so and offers nothing to set.

## Set an alarm

Open **Wake-up** and choose **Add an alarm**.

1. Give it a name, or leave it: the default is « Wake-up 07:00 », following the
   time until you type one.
2. Set the hour and the minute, and pick the weekdays. Monday to Friday is
   ticked at first. With no day picked, the alarm cannot be saved.
3. Pick the **playlist** (Favorites first) and whether to shuffle it.
4. Pick the **output**: the Nidoo speakers, or any output Pinson lists. The
   Nidoo speakers are chosen first when Pinson lists them.
5. Pick the **fade-in**: Off, 30 s, 1 min, 2 min or 5 min. The sound rises from
   silence to the set volume over that time.
6. Check the **next alarm** line, then **Save**.

The list shows each alarm with its time, weekdays, playlist, speakers, fade-in
and next ring. The switch on a row turns that alarm on or off.

## Speakers

When the output is the Nidoo speakers, the editor lists every speaker with a
music volume:

- A **ticked** speaker is unmuted and set to the volume of its slider before
  the music starts.
- An **unticked** speaker is **muted** during the alarm, and stays muted after
  it: the alarm does not unmute it again. Untick the speakers of the rooms where
  nobody should wake up.

If a speaker is later removed or renamed, the alarm keeps working with the
speakers it still finds. The editor drops the ones that are gone the next time
you save.

## If the playlist is gone

If a saved playlist is no longer listed by Pinson, the editor shows it as
« unknown playlist » and keeps it when you save. At the time of the alarm,
Pinson plays its Favorites instead.

## A spoken backup

Pinson may be offline when the alarm rings. The page does not add a backup,
but because the alarm is an ordinary flow you can open it from the flows list
(**open** on the « Managed by Wake-up » line takes you back to the page) and
add a `speak` card on a satellite in a copy of the flow. Do not edit the
managed flow itself: the page rewrites it on the next save, and Nidoo asks
before saving a managed flow from the canvas.

## A broken alarm

If someone edits an alarm's flow by hand and its settings can no longer be
read, the list shows it as **Unreadable alarm** with a **Delete** button only.

## Turn it off

Turning Wake-up off in Settings hides the menu entry and switches **every**
alarm off. If some are on, Nidoo asks first and says how many will be turned
off. Turning Wake-up on again changes no alarm: they stay off until you switch
them on one by one.
