---
title: Flows (automations)
sidebar_position: 5
description: "Build automations from cards on a canvas. When a device changes, a time comes or the sun sets, check a condition and act: switch a light, send a notification or speak in a room."
---

A flow is an automation: when something happens, and a condition holds, it does something. For example, the hall light comes on when motion is detected, or the shutters close at sunset.

You build a flow from cards on a canvas, in **Flows**. Each card is one step: a trigger that starts the flow, a check, a wait or an action. You can also ask the assistant to write one for you. See [Talking to the house (MCP)](assistants-mcp.md).

## Your first flow

Let's build a flow that turns a light on when motion is detected, and turns it off five minutes after the last motion.

1. Open **Flows** and press **New flow**.
2. Type the motion sensor's name in the card library's search, and add its **When … turns on** card. (Or press **Device** in the **WHEN** group, pick the sensor, and set **Fires when it becomes** to **Detected**.)
3. Search for your lamp in the card library. Add its **Turn on** card.
4. Add a **Wait** card and set it to 5 minutes.
5. Add the lamp's **Turn off** card.
6. Drag the cards where you like, then wire them in order: the motion card, **Turn on**, **Wait**, **Turn off**. Drag from the dot on the right of one card onto the next.
7. In the title bar, set the mode to **Restart if running**. New motion during the five minutes starts the count again, so the light stays on until five minutes after the last motion. See [When a flow is already running](#when-a-flow-is-already-running).
8. Give the flow a name, pick a room, and press **Save**.

![Building the flow: a search for "entrée" adds When Entrée turns on, a search for "led" adds Turn on Led, then a Wait 5 min and Turn off Led; each card is dragged into place, then wired by dragging from its output dot onto the next card](/img/screens/flow-build.webp#gh-dark-mode-only)
![Building the flow: a search for "entrée" adds When Entrée turns on, a search for "led" adds Turn on Led, then a Wait 5 min and Turn off Led; each card is dragged into place, then wired by dragging from its output dot onto the next card](/img/screens/flow-build-ivoire.webp#gh-light-mode-only)

The first save creates the flow switched off. Turn on its switch at the top right of the flow page. Then press **Run now** to test it. The canvas lights up the cards as the run passes through them (see [Watching runs](#watching-runs)).

## The cards

The card library has five groups: **WHEN**, **AND**, **THEN**, **WAIT** and **FLOW**. Each card below is one step.

### WHEN: what starts a flow

| Card | What it does | See |
| --- | --- | --- |
| **Device** | When a device changes or sends an event | [The Device card](#the-device-card) |
| **Schedule** | At a set time, or on a repeat | [Schedule and sun cards](#schedule-and-sun-cards) |
| **Sun event** | Sunrise, sunset, dusk or dawn | [Schedule and sun cards](#schedule-and-sun-cards) |
| **Off-peak** | When off-peak starts or ends | [Off-peak and alarm cards](#off-peak-and-alarm-cards) |
| **Alarm** | When the alarm changes mode | [Off-peak and alarm cards](#off-peak-and-alarm-cards) |
| **Silent for** | When an entity stops reporting | [Check that a sensor is alive](#check-that-a-sensor-is-alive) |
| **Manual** | Start by hand from the ▶ button | [Manual trigger](#manual-trigger) |

### AND: checks that branch

| Card | What it does | See |
| --- | --- | --- |
| **When it's dark** | Between dusk and dawn | [If cards](#if-cards) |
| **Is it off-peak?** | Off-peak hours right now | |
| **Switch** | Several exits, one per rule | [Branch on several values](#branch-on-several-values) |
| **Math** | Compute a number from your devices | [Compute a value](#compute-a-value) |
| **Time of day** | Inside or outside two times | [Only at certain hours](#only-at-certain-hours) |

### THEN: actions

| Card | What it does | See |
| --- | --- | --- |
| **Notify** | A push to your phone | [Values in a notification](#values-in-a-notification) |
| **Speak** | Said out loud on the speakers | [Speak in a room](sound.md#speak-in-a-room) |
| **Ask** | Asks out loud and waits for the answer | [Ask a question](#ask-a-question) |

Device actions, such as **Turn on** or **Close**, are also in this group. See [Device actions](#device-actions).

### WAIT: pauses

| Card | What it does | See |
| --- | --- | --- |
| **Wait** | A fixed or random delay | |
| **Limit** | Lets only so many runs through | [Check that a sensor is alive](#check-that-a-sensor-is-alive) |

A **Wait** card waits for a set time, such as 90 seconds, 5 minutes or 1 hour 30 minutes. With **Up to (random)**, it waits a random time between the two values.

### FLOW: act on other flows

| Card | What it does | See |
| --- | --- | --- |
| **Disable this flow** | Stops it from running again | |
| **Enable a flow** | Switches another flow on or off | |
| **Pick a label** | Pick a season, or turn a label's flows on or off | [Labels and choices](#labels-and-choices) |
| **Back up** | Backs up the hub to the FTP server and waits for the result | [A nightly backup](backup.md#a-nightly-backup-with-a-flow) |

The **Back up** card branches on the outcome: **ok**, **partial** (saved without the Zigbee2MQTT data) or **failed**.

Search the library for a device, a room or an action word to find more cards. Type a device name, such as **Front door**, and you get its own cards: **When Front door changes**, one card per value it reports, plus its checks and actions. Type a room name to list every device in that room.

An action word works in your language. `close` (or `ferm` in French) lists the cards that close a shutter. `close south` lists only the south shutter's cards. Several words narrow the list. Hidden devices are listed; disabled, diagnostic and folded ones are not.

## Setting up a card

Pick devices with the [entity picker](web-ui.md#picking-an-entity), not by typing their names.

### The Device card

Pick any device: a door or motion sensor, a plug, a remote, a tag reader. Then choose the settings:

- **Fires when it becomes**: **Any change**, or one of the values the device reports, such as **Open**, **Closed** or **Detected**.
- **Only after**: the previous value. The card fires only when the device changes from that value.
- A remote, an event or a tag shows its buttons as chips. Pick one, or **any** for every press.

A **Device** card with a value under **Fires when it becomes** also fires when that value first appears after being unknown. This is useful for a sensor that reports its alarm field only once it's set.

### Hold a value or cross a threshold

Two settings on the Device card turn it into a condition that has to last, or a threshold to cross.

- **For (stays that way)**: the card fires only once the condition has held that long, from 1 second to 168 hours. "No motion for 5 minutes" is a motion sensor set to **Clear**, with **For** set to 5 minutes. A report that breaks the condition cancels the wait. With no value set, the device must stay unchanged for that long.
- **Goes above** and **Goes below**: for a device that reports a number. The card fires once when the value crosses the threshold, not on every reading beyond it.
  - With **Goes above** at 25, a reading going from 24.8 to 25.3 fires. The next readings, 25.6 and 26, do not.
  - The value must come back to 25 or under before it can fire again.
  - Set both to fire when the value enters the range between them, for example 18 to 22 °C.
  - Exactly 25 does not count as above 25.
- The two thresholds cannot be combined with **Fires when it becomes** or **Only after**. Setting one clears the other.

The card has no margin. A reading that hovers around the threshold fires on every crossing. On a noisy sensor, add a **For** time, such as above 25 for 10 minutes, so it has to settle first.

A wait in progress is lost when the hub restarts. After a restart, the card fires only on the next change that meets the condition.

### Value of

The When, If and Switch cards read the device's main value: on or off, a measurement, or a state such as **Cleaning**. The **Value of** menu lists everything else the device reports, with each current value:

- the device's other fields: a vacuum's **Battery** and **Fan speed**, a light's **Brightness**, a cover's **Position**, a climate device's current and target temperature;
- the device's other entities, diagnostic ones included: a smoke detector's **Battery**, **Battery low** or **Link quality**.

To run something when a vacuum's battery is low, add an **If** card on the vacuum. Pick **Battery (62 %)** in **Value of**, then **≤** and 55. The card reads "Roborock · Battery ≤ 55 %?".

- A Zigbee device's battery is an entity of its own. Picking it in **Value of** sets the card's device to that battery.
- A **Device** card on a field fires only when that value changes. A battery trigger ignores the vacuum going from Cleaning to Docked. A trigger on the vacuum ignores battery reports.
- An **If** or **Switch** card with no device of its own compares the value of the trigger before it.
- Picking another device resets **Value of** to the main value and clears the compared value.
- **Average over** works only on fields that the history records, such as a cover's position or a climate's current temperature. For other fields, such as a vacuum's battery, the option is hidden.

### If cards

An **If** card compares a value with a number, text or on/off, and has two exits: **yes** and **no**. Its settings:

- **Value**: the list of what the device can report, in your language. A shutter offers **Open**, **Closed**, **Opening** and **Closing**. A sensor offers on and off. A climate device offers its modes. A measurement gets a number box with its unit.
- **Other value…**: anything not in the list: a number, true or false, or text.
- **Trigger value**: the card compares the value of the trigger before it. The list follows that trigger.
- **Average over**: compares the average of the readings over a window, such as 15 min, instead of the current value. The newest reading counts. Leave it empty to use the current value. The longest window is 24 hours. For example, a temperature whose 30-minute average is 19 or less, not one noisy reading.
- **Shift (minutes)**, on **When it's dark**: moves the dark window. **−30** makes it dark 30 minutes before civil dusk, and keeps it dark until 30 minutes after civil dawn.

**When it's dark** is true between dusk and dawn at your home location.

### Branch on several values

A **Switch** card has one exit per rule, up to 10 rules. One card can sort a value into bands. For example, three rules on a temperature: below 18, between 18 and 22, and above 22, with **otherwise** turned on.

- Each rule compares the value: equal, not equal, above, below, at least, at most, or **between**. A **between** rule includes both ends.
- The exits are numbered 1, 2 and so on, in rule order. **otherwise** is an extra exit that you turn on.
- By default the first matching rule wins. Turn on **Fire every matching rule** to follow every rule that matches.
- Like an **If** card, a **Switch** can read another device and use **Average over**.
- Add, reorder or delete rules in the card's settings. Each rule keeps its own labelled exit. The wires follow their rule when you reorder or delete rules.
- An unknown value goes to **otherwise**, or nowhere when **otherwise** is off. A not-equal rule that matches the value wins over that.

### Only at certain hours

A **Time of day** card sends the run to **in** or **outside**, depending on the time.

- **From** is included. **Until** is not.
- A range that ends before it starts crosses midnight. From 22:00 until 06:00 covers the night.
- Times use 24 hours, in the house time zone.
- The card needs no device, so it works after a **Schedule** or **Sun event** card.

The canvas labels its outputs with the range, such as "07:00–22:30", and "outside".

### Schedule and sun cards

- **Schedule** opens a picker with these choices: **Every N minutes**, **Hourly**, **Daily**, **Weekdays**, or **Advanced** for a raw cron expression (minute, hour, day, month, weekday). The card lists its **Next fires**: the next three times it runs.
- **Sun event** fires at sunrise, sunset, dusk or dawn, at **At the event** or moved **Before** or **After** by some minutes. The card shows when it fires next.
- Schedules use the house time zone: the time zone of your home location, set in the setup wizard or **Settings → Location**. Without a home location, the hub's system time zone is used.
- Sun cards and **When it's dark** also read the home location. Without one, the hub uses Paris and the card shows a **Paris fallback** badge. See the [Settings reference](../reference/configuration.md#home-location).

### Off-peak and alarm cards

- **Off-peak** fires when **Off-peak starts** or **Off-peak ends**. Off-peak times come from the off-peak device, or from the off-peak hours in **Settings → Home**.
- **Is it off-peak?** is a check: it is true during off-peak hours.
- **Alarm** fires when the alarm changes mode. Set **Fires when it becomes** to the new mode, **Only after** to the mode it comes from, or both. **Any mode** matches every change. With only **Only after** set, the card reads "When the alarm leaves Disarmed".

### Device actions

Search for a device and add one of its action cards, such as **Turn on** or **Close**. Each action card has these settings:

- **Action**: what the device accepts. Each action has its own fields: a position or temperature, a light's brightness, colour and effect, or an alarm code when the panel needs one.
- **Advanced (JSON)**: at the end of the list. Use it to type a command by hand.

The lists come from what the device announces about itself when it joins the hub.

### Manual trigger

A **Manual** card starts a run when you press its ▶ button. Use it to test a branch, or to run a flow by hand.

- Its optional **Value** is **No value**, **On**, **Off**, a number or text. The run carries that value.
- An **If** or **Switch** after a card with a value compares that value, not a device. It cannot use **Average over**, because there is no device to average. After a card with no value, an **If** or **Switch** needs a device, as after a schedule or sun card.
- In a notification, **{trigger}** prints the value. See [Values in a notification](#values-in-a-notification).
- The Runs drawer shows "▶ manual · on", or "▶ manual" when there is no value.
- When a run is already active, a press follows the flow's mode. **Ignore if running** refuses the press ("a run is active"). **Restart if running** cancels the active run and starts again.
- **Run now** is refused while a run is active, in both modes.
- A flow with several **Manual** cards can run from any of them. Press ▶ on the card you want.
- The ▶ button is greyed out while you edit ("Save to run from here"), and on a disabled flow. Save the flow first.
- A disabled or invalid flow refuses presses, as **Run now** does.

## Values in a notification

A **Notify** card's title and body can show live values. Under each field, press **＋ Value**, pick a device, and a placeholder is inserted at the cursor. The **Preview** line under the body shows the message with the house's current values. The hub fills in the placeholders when the step runs.

| Template | Sent in English | Sent in French |
| --- | --- | --- |
| `{weather_temperature}` | `10.1 °C` | `10,1 °C` |
| `{weather_temperature:0}` | `10 °C` | `10 °C` |
| `{weather_today_max - weather_today_min:1}` | `5.4 °C` | `5,4 °C` |
| `{weather_tomorrow_precipitation * 2}` | `0 mm` | `0 mm` |
| `{weather_condition}` | `Cloudy` | `Nuageux` |
| `{trigger}` | the value that started the run | the same |
| `{{` and `}}` | a literal `{` and `}` | the same |

- A placeholder reads a device's value with its unit. `:N` rounds to N decimals, from 0 to 6. Without it, a number shows at most 2 decimals.
- Maths uses `+ - * /` and parentheses. Put spaces around the operators: `a-b` names one device, while `a - b` subtracts.
- `{name}`, where `name` is the result name of a **Math** card, prints that result, with no unit. `:N` rounds it. See [Compute a value](#compute-a-value).
- On/off and text values are translated: open and closed, detected and clear, weather conditions, and cover, alarm and climate states.
- A value that is unknown when the step runs prints `?`, and the push is still sent.
- `{trigger}` prints `?` in a run that no state change started, such as a schedule, the sun, or **Run now**. In a run started by a **Manual** card with a value, `{trigger}` prints that value, with no unit.
- The language is the web app's language when you save the flow.
- Text that does not parse, or a placeholder naming a device the house does not have, is refused when you save.

## Compute a value

A **Math** card turns house values into one number. Here is an example: a water heater that should use only the solar surplus. It may draw what it already draws, plus what the meter exports, less a 100 W margin.

A **Math** card has four settings:

- **Inputs**: the readings the formula uses. Give each one a name. Pick **now** for the current value, or an average over 1 min, 5 min, 15 min or 1 h. In the example, `grid` is the power at the meter, negative when the house exports. `heater` is the heater's power averaged over 1 minute, so one reading does not move the result.
- **Formula**: `clamp(heater - 100 - grid, 0, 1650)` keeps the surplus between 0 and 1650 W. The formula takes numbers, input names, `+ - * / %`, comparisons, `and`, `or` and `not`. It also has the functions `min`, `max`, `abs`, `floor`, `ceil`, `round`, `clamp`, `if` and `interp`, a lookup table for a curve. The [formula grammar](../reference/flow-nodes.md#formula-grammar) lists every rule.
- **Result name**: `desired` in this example. Later cards use the result under this name.
- **Save to (optional)**: writes the result to a number, rounded to its step and kept in its range. Leave it empty to use the result only inside the run.

To build the water heater flow, card by card:

1. Add a **Device** card for the grid meter, set to **Any change**.
2. Add an **If** card on the tank's temperature sensor. Choose **>** and type 72. Under **When the value is unknown**, choose **Take Yes**. A tank sensor with no reading then counts as too hot, so the flow fails safe and never heats blind.
3. On the **yes** exit, add an action that sets the virtual number **Heater target** to 0.
4. On the **no** exit, add a **Math** card with the four settings above. Save to **Heater target**. Create that virtual number first, in watts from 0 to 1650. See [Groups and virtual entities](groups-and-virtual-entities.md).
5. Wire the cards: the meter goes to the **If** card, its **yes** goes to the action, and its **no** goes to the **Math** card.

A heater that takes a 0 to 100 % setting needs the watts converted to a percentage first, with an `interp` step in the formula.

Later cards in the same run can use the result:

- An action card: in its **Action** settings, choose **Value from result** and the result name. The number typed in the action is then ignored. This works for set number, set temperature, set position and brightness.
- An **If** card can compare the result instead of a device.
- A notification prints it: `{desired:0}` shows the result with no decimals.

A result lasts for one run. A card that reads a result must come after the **Math** card on the same path. Otherwise the run stops with the error `no result desired in this run`.

To remember a value between runs, save it to a virtual number. Then add that number as an input to the next run's **Math** card, and smooth the new value with the previous one in the formula.

## Ask a question

An **Ask** card speaks a question on a satellite and waits for the answer. For example, when someone comes home, ask in the living room: "Shall I turn the heating up?" The answers are **Yes** and **No**.

1. Add a **Device** card on the front door, set to **Any change**.
2. Add an **Ask** card. Set **Question** to "Shall I turn the heating up?" and **Room** to the living room. Add the answers **Yes** and **No**.
3. On **Yes**, add a climate action that raises the target temperature.
4. Leave **No** unwired, so nothing happens on that answer.
5. Leave **No answer** unwired too, or connect it to whatever should happen when nobody replies.

The card has one exit per answer, in order, plus **No answer**. Leave an exit unwired to do nothing on it.

- Each answer has a label, which is matched against what was heard.
- **Also accepts** lists other ways of saying the answer, separated by commas, such as "sure, okay".
- When two answers could match, the longest phrase wins. "Yes but no thanks" goes to the answer that holds "no thanks", not to **Yes**.
- A leading article such as "un" or "one" is not read as a number.
- An answer can carry a number, for example "set it to 21". Name it with **Keep the number as**. Use lowercase letters, digits and underscores, starting with a letter. Later cards use it like a **Math** result, for example a climate action with **Value from result**.
- **If no number** is the value used when the answer holds no number. It appears only when a name is set. Clearing the name clears it too.

How it behaves:

- Only satellites that can both speak and listen are asked. A satellite busy in an Assist exchange is skipped.
- With no **Room**, one satellite per room is asked, and the first answer wins.
- Silence leaves by **No answer** at once.
- An answer that was heard but matches nothing gets one retry, on the satellite that heard it. Then it leaves by **No answer**.
- The whole step, retry included, takes 45 seconds at most.
- With no speech input, or no satellite to ask, the card leaves by **No answer**. The run log says why. This is not a failed step.
- The card needs [speech input](sound.md#talk-to-a-satellite) on a satellite with a microphone.

## Check that a sensor is alive

A smoke or battery sensor is useful only while it is alive. A **Silent for** card starts a run when a device has sent no report at all for a set time, whatever its value.

Add a **Limit** card after it, so a sensor that stays dead does not send an alert every hour:

1. **Silent for**: pick the smoke sensor and set the time to 50 minutes.
2. **Limit**: set **Runs allowed** to 4 and **Per window** to 24 h.
3. **Notify**: set the title to "No report for {trigger}" and the severity to **warning**. `{trigger}` prints the time of silence.

Rules for these cards:

- The card fires once per silence. It fires again only after the device reports and then goes quiet for the time again.
- The clock starts when you save or enable the flow, not from the device's last report. Enabling a check on a sensor that has been quiet for hours waits the full time first.
- While the hub's connection is down, nothing can be reported, so the clocks stop and restart when the connection returns. A connection that drops more often than the set time keeps restarting them.
- **Limit** lets at most **Runs allowed** through in any rolling **Per window**. Past that, the run ends there. Every run that reaches it counts, even one cancelled later or started by hand. Put it right before the action. The window slides, so there is no reset at midnight.
- The silence clocks and the **Limit** counts are kept in memory. A hub restart starts them again.

## Labels and choices

A **label** is a name you give to a group of flows, such as "Winter" or "Holidays", with a colour. A flow can carry several labels. Labels only group and switch flows. They do not change what a flow does.

A **choice** is a set of two or more labels where one is current, such as "Season: Winter | Summer". A label belongs to at most one choice. A label in no choice is standalone.

- **Picking** a label in a choice turns on every flow carrying it. It turns off every flow carrying another label of that choice. The picked label becomes the current one. Flows with no label of the choice are left alone. A flow with the picked label and another one is turned on.
- **Turning a label on or off** switches every flow that carries it, whether it is in a choice or not.

A switch acts once. You can still turn any flow on or off by hand, and it stays that way until the next switch. Only flows not already in the right state are changed. Invalid flows are skipped. The message names every flow it could not switch, for example: "Summer picked: 5 on, 12 off. 1 not changed: Old heater (invalid)."

### Winter and summer

1. On **Flows**, set **Group** to **Label**, then press **＋ Label** on the "No label" section. The label manager opens.
2. Press **＋ Choice** and name it "Season". Then press **＋ Label in this choice** twice, for "Winter" and "Summer".
3. Tag your heating flows "Winter" and your cooling flows "Summer". Use the **⋯** menu on a row, under **Labels**, or the **Labels** field next to the room in the flow editor.
4. The "Season" section header now shows **Winter | Summer**. Press **Summer**. The cooling flows turn on, the heating flows turn off, and a message gives the counts.

To switch to summer on 21 June by itself, make a flow:

1. Add a **Schedule** card. Choose **Advanced** and enter `0 0 21 6 *`, which means midnight on 21 June, every year.
2. Add a **Pick a label** card, set to the Season choice and **Summer**.
3. Wire the **Schedule** card to the **Pick a label** card.

Make a second flow on `0 0 21 12 *` to pick **Winter** back. The **Pick a label** card shows a live preview of what it would switch now. The run log gives the counts. A label action can turn off its own flow. The run still finishes its steps.

### The label manager

The label manager lists each choice with its labels, then the standalone labels. Each line shows its colour, its name, and how many flows carry it. Open it with **⚙** on a label or choice section, or with **＋ Label** on "No label".

- **✎** renames a label, changes its colour, or moves it into or out of a choice. A new label gets the least-used colour.
- **🗑** deletes a label after asking. It is removed from every flow carrying it. If a **Pick a label** card names it, the question lists those flows. Their card then fails when it runs, and the flow cannot be saved until the card is fixed.
- Deleting a choice keeps its labels, as standalone labels.
- A choice with fewer than two labels shows "Needs two labels" and cannot be picked yet.
- Nothing in the manager switches flows. Renaming a label does not break the flows that use it.
- The assistant can read and switch labels too. See [MCP tools](../reference/mcp-tools.md).

## When a flow is already running

Each flow has a mode, set in the title bar in edit mode. It decides what happens when the trigger fires during a run.

- **Ignore if running** (the default): the new trigger is ignored, and the run is recorded as skipped.
- **Restart if running**: the active run is cancelled, and a new one starts.

**Restart if running** is what makes "5 minutes after the last motion" work. Motion coming back cancels the pending wait.

## Watching runs

While a run is in progress, the canvas follows it live. Each card it reaches lights up, a dashed wire runs into the card it is on, and a **Wait** card counts down ("0:02 left"). On a check, only the exit it took lights up.

![A run on the canvas: the ▶ of a Manual card is pressed, the Manual card lights up, a dashed wire runs into Wait 2 s while it counts down, then the If card "Colder than 10 °C?" lights up and its green yes wire runs into Wait 3 s, while the no branch stays dark](/img/screens/flow-run.webp#gh-dark-mode-only)
![A run on the canvas: the ▶ of a Manual card is pressed, the Manual card lights up, a dashed wire runs into Wait 2 s while it counts down, then the If card "Colder than 10 °C?" lights up and its green yes wire runs into Wait 3 s, while the no branch stays dark](/img/screens/flow-run-ivoire.webp#gh-light-mode-only)

- Press **Runs** at the top right of the flow page to open the recent runs.
- Pick a run to replay it on the canvas. The cards and wires it passed through light up, and the rest dim. Esc, or closing **Runs**, stops the replay.
- Failed steps show in red.
- A failing card stops only its own branch. The cards after it do not run, but the other branches still do. A trigger wired to five commands still sends the other four when one device does not answer. The run then ends as **failed**.
- A card with no wire leading into it never runs.
- A scheduled run that comes while the hub has lost its connection is recorded as skipped, because its commands could not be sent.
- **Run now** and manual runs are refused when the flow is disabled, invalid, or already running.
- The hub keeps the most recent runs of each flow. The default is 20 (see the [Advanced settings](../reference/configuration.md#advanced)).
- A hub restart forgets the runs. The flows themselves stay.

## The Flows page

Open **Flows** to see your flows. The page has these parts:

- **New flow** starts a new flow.
- The search box finds flows by name, description, trigger, room and label names. It ignores case and accents.
- **Sort** by **Name** or **Last run**. **Group** by **Room**, **Label** or **None**. Switch between **List** and **Cards**.
- Status chips, each with a count: **All**, **On** (enabled and valid), **Off**, and **⚠ Needs attention** (invalid, or the latest run failed).
- The **Trigger ▾** menu lets you tick any kinds of trigger: **Device state**, **Schedule**, **Sun**, **Off-peak**, **Alarm**, **Silent for**, **Button / event** or **Manual**. The button shows how many are ticked.
- Each row shows the trigger icon, the name, a **restart** tag for flows in restart mode, and a red **Invalid** tag for a flow that does not validate. Hover over the tag to see why. A flow needs at least one trigger card and no loops. An invalid flow never runs. The row also shows its labels, a trigger summary, its last run ("Done · 2 min ago", or "No runs yet"; failed runs in red), the enable switch and the **⋯** menu.
- Rooms are sections, with **No room** last. Fold a section to hide it. A folded section that hides a flow needing attention shows ⚠.
- Grouped by label, each choice is a section whose header picks the current label. Each standalone label has **Turn all on** and **Turn all off** in its header. **No label** comes last. A flow with several labels appears in each of their sections.
- The **⋯** menu has **Open**, **Move to room**, **Labels** and **Delete flow…**.
  - An invalid flow cannot be moved.
  - **＋ New label…** creates a label and tags the flow with it.
  - **Delete flow…** asks first, and a deleted flow cannot be brought back.
- The view, grouping, sort, filters and folded sections are remembered in each browser. The search is not.

![The Flows page: a search box, sort, group and view toggles, filter chips, and flows grouped by room with an enable switch and a ⋯ menu on each row](/img/screens/flows.webp#gh-dark-mode-only)
![The Flows page: a search box, sort, group and view toggles, filter chips, and flows grouped by room with an enable switch and a ⋯ menu on each row](/img/screens/flows-ivoire.webp#gh-light-mode-only)

A flow can sit in a room. The room only decides where the Flows page lists the flow. It does not change what the flow does.

- A flow whose room was deleted keeps that room, and it never blocks a save.
- Invalid flows keep their real name and room in the list.

## The canvas and editor

### Viewing a flow

- The top left of the flow page has a back arrow, the flow's name, its mode tag (**single** or **restart**), and its room.
- The top right has the enable switch, **Run now**, a pencil to edit, and **Runs**.
- Zoom buttons sit at the bottom left, and a minimap at the bottom right. The scroll wheel pans. ⌘/Ctrl plus the wheel, or a pinch, zooms.
- Each card is a pill with an icon and a sentence, such as "Motion at Entrée", "Turn off LED" or "Wait 5 min". Wires go around the cards.
- The two outputs of an **If** card are labelled green **yes** and red **no**. A **Switch** labels each output with its rule, such as "< 18", or "otherwise".

![A flow on the canvas: a header with the flow's name, mode and room, the enable switch, run, edit and Runs buttons at the top right, zoom buttons bottom left and a minimap bottom right](/img/screens/flow-canvas.webp#gh-dark-mode-only)
![A flow on the canvas: a header with the flow's name, mode and room, the enable switch, run, edit and Runs buttons at the top right, zoom buttons bottom left and a minimap bottom right](/img/screens/flow-canvas-ivoire.webp#gh-light-mode-only)

### Editing

Press the pencil to edit. It is disabled on an invalid flow.

- Click a card to select it. A card with an amber ring is selected, and its settings open in the side panel.
- Drag a card to move it.
- Drag from an output onto another card to wire them.
- Drop a card that has no wires onto a wire to insert it there. The wire turns green first. On an **If** card, the flow goes on through the **yes** output.
- Click a wire, then its ✕, to remove it.
- Press ⌫ to delete the selection. Deleting a card between two others wires them together again.
- ⌘/Ctrl + D duplicates the selected card.
- ⌘/Ctrl + Z undoes. Add Shift to redo.
- Esc clears the selection, or stops a replay.

### The card library

In edit mode, the right panel holds the card library, grouped **WHEN**, **AND**, **THEN**, **WAIT** and **FLOW**.

- Click a card, or drag it onto the canvas, to add it. The library stays open, so you can add several.
- Type a device name or a room in the search to list the triggers, checks and actions of every matching device, sorted by name. Case and accents don't matter, and several words narrow the list.
- Enter adds the highlighted card. ⌘/Ctrl + Z right after takes it back out, even though the search box still has the focus.
- Select a card on the canvas to show its settings in the panel. **Close** brings the library back.

### The top bar

- The flow's name, and its room picker.
- The mode: **Ignore if running** or **Restart if running**.
- **Tidy** lays the cards out again, leaving more room around a card wired to several others.
- Undo, **Cancel editing** (which discards your changes), and **Save**.
- **New flow** opens an unsaved draft. The first **Save** creates the flow, switched off.
- Nothing reaches the hub until you press **Save**.

### On a phone

On a phone or a touch-only screen, the canvas is view-only. The pencil and **New flow** are hidden. You can still pan, pinch to zoom, open the side panel, use the enable switch, and press **Run now**.

## Going further

- Flows are stored on the hub's message broker, as JSON documents, one per flow.
- Every card is described in [Flow nodes](../reference/flow-nodes.md).
- The assistant writes flows with the `set_flow` tool. See [MCP tools](../reference/mcp-tools.md).
- The REST routes for flows are in the [HTTP API](../reference/http-api.md). Weather values, such as the outdoor temperature, are entities you can use in cards. See [Weather](weather.md).
