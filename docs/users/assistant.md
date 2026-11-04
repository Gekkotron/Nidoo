---
title: Assistant
sidebar_position: 19
description: Short French sentences typed on the Assistant page switch lights, set heating setpoints and read temperatures, with safety checks, a Simulation switch and a Faux button.
---

The **Assistant** understands short French sentences about the house and acts on them or answers, with a one-line reply. Open it from **Assistant** in the sidebar, or from the round button in the bottom-right corner of every page.

It runs in the hub with fixed rules, no language model: the same sentence always gives the same result, instantly, and nothing leaves the house. It is deliberately limited. When it is not sure, it asks or does nothing; it never guesses.

## What it understands

Rooms are your own rooms, by name ("salon", "la chambre de Léa", "chez Léa", "Léa"). Accents, capitals and a one-letter typo in a long word don't matter.

**Lights**, on or off, for a room or for named lights in it:

- « allume la lumière du salon », « éteins le salon », « éteins tout dans le bureau »
- « éteins la led du salon », « allume la lampe de la chambre »

A room's lights are the ones a mood's "lights" action uses: every light of the room, the ones hidden from the dashboard included, but not groups. Lights already in the wanted state are left alone; an offline light gets no command and the reply says it did not answer.

**Heating setpoints**, for a room whose [climate](climate.md) zone is active:

- an exact value: « mets le salon à 20 », « règle la chambre de Léa à 19,5 degrés »
- a change from the current setpoint: « monte le chauffage d'un degré au bureau », « baisse un peu le chauffage du salon » (a bare "monte" or "baisse" is 1 °C, "un peu" is 0,5 °C), « chauffage +2 au bureau »

A change starts from what the room's devices are really set to, a temperature set by hand included. If they disagree (two units, one changed by hand), the assistant names the values and asks for a temperature. When the room's current reading is fresh and already past the setpoint, a raise starts from the reading if it is higher, and a lower from the reading if it is lower: with a setpoint of 15,5 °C in a room at 18 °C, "monte" gives 19 °C.

The setpoint holds until the zone's next scheduled change, at most the zone's manual hold (4 h by default): the same hold a change made by hand on the device gets. It replaces a temperature set by hand on a unit of the zone, so every unit follows it. "Allume le chauffage" alone is not a setpoint; the assistant asks for a temperature.

**Temperatures**: « quelle température fait-il dans le salon ? », « il fait combien au bureau ? », « il fait combien dehors ? ». A room's temperature is its climate zone's reading, else its thermometer; outdoors is the climate engine's outdoor sensor, else the weather. An old reading is said with its age.

## Safety

- **A statement or a question is not an order.** "Il fait 18 degrés au salon", "la lumière du salon est allumée ?" or "qui a éteint le salon ?" never act (a temperature question is answered). A polite request still is one: « peux-tu allumer la lumière du salon ? ».
- **Not sure, no action.** Two opposite orders ("allume et éteins"), a negation ("n'éteins pas"), a percentage ("à 50 %") or a device it does not handle yet (shutters, plugs) give "Je n'ai pas compris", with a few example sentences built from your rooms.
- **Ambiguous room, a question.** "La chambre" with two bedrooms asks "Dans quelle pièce ?" with the rooms as buttons. You can also type the answer ("Léa").
- **Unusual setpoint, a confirmation.** A setpoint outside **16–23 °C**, or more than **3 °C** away from the room's current temperature, is asked first: "Passer le Salon à 25 °C ?" with **Oui** / **Non** (typing "oui" or "non" works too). A yes applies exactly the value asked about. The 16–23 °C range is set by `assist_min_c` and `assist_max_c` in the [home settings](../reference/http-api.md#home-settings).
- **Out of the device's range, a refusal.** A value the heaters cannot take is refused with their range, never clamped silently.
- A question waits **60 seconds**. Any other sentence cancels it.

Every reply names the room, what was done and the value, for example "Chambre Léa réglée à 19 °C (avant 17,5 °C)." When a light does not answer, the reply says which one.

## Simulation and Faux

The **Simulation** switch under the field plans and replies but sends nothing to the house; replies start with "Simulation :". It is the safe way to try phrasings. The page remembers it per browser.

The panel next to the conversation, **How it understood**, shows the selected turn: the normalised words, the intent, the room and how it was found, the devices, the value and its arithmetic ("19 − 1 = 18 °C", or "mesure 18 > consigne 15,5 : 18 + 1 = 19 °C" when the reading was the start), the safety check, each action with its result, and the time taken. **Faux** marks a turn as wrongly understood, so it can be found in the log later.

With the history database, turns are kept 90 days. Without it, the hub keeps the last 100 turns in memory and the page says history is not kept.

## Voice

The hub can turn text into French speech, on its own processor, with no cloud call when it speaks. Open it with the gear button at the top right of the Assistant page, which leads to **Assistant settings**. For now this is a capability, not a habit: **Assist does not speak its replies yet**. Who speaks when is decided once satellites (microphone and speaker boards) exist.

The engine is [Phonon](https://huggingface.co/gradium) by Gradium. The hub never ships the model files: you download them yourself, under their licence.

### Licence and token

The French models, `gradium/phonon-40m-fr` and `gradium/phonon-40m-multilingual`, are **gated** on Hugging Face and licensed **CC BY-NC 4.0**: non-commercial use only. To use them:

1. On the model's page on huggingface.co, accept the terms.
2. In your Hugging Face account, open **Settings, Access Tokens** and create a token with the **Read** role.
3. Paste it in **Hugging Face token** at the bottom of the settings screen and save. The field is masked and write-only: the hub never shows the token again. **Clear** removes it. Changing it does not restart the hub, and clearing it keeps the models you already installed.

The token is sent only to huggingface.co.

### Adding a model

Under **Add a model**, paste the model page address (`https://huggingface.co/owner/repo`, `hf://owner/repo` or `owner/repo`) and press **List files**. The hub shows the weight files the repository offers, with their sizes:

| File | Size (40m models) | Notes |
| --- | --- | --- |
| `model.q8.gguf` | about 75 MB | The default pick. |
| `model.q4k.gguf` | about 60 MB | Smaller. |
| `model.safetensors` | about 122 MB | Full precision. |

Choose one and press **Download**. It runs in the background with a progress bar and a **Cancel** button; the model appears in the list only when every file is complete and checked. One download runs at a time. If the repository is gated and no token is saved, or the account has not accepted the terms, the screen says so and links to the model page. A restart of the hub drops an unfinished download.

No internet on the hub, or a token you would rather not use? Press **Upload from this computer** and pick the model's files (the same files, from a download made elsewhere). The hub checks that they make a loadable model before keeping them.

### Choosing the model and the voice

**Installed models** lists each model with its weight file, size, short commit, licence and whether it speaks French. Select **Active** on one to use it; only the active model is loaded in memory, and switching unloads the previous one first. **Remove** deletes a model (after a confirmation); removing the active one leaves none active. A model that does not speak French can be installed, and the screen warns you.

**Speech** at the top shows the engine's state: No model, Loading, Ready or Error with a message. A broken or missing model never stops the hub or text Assist: speech just shows an error.

**Voice** lists **Default**, the model's own voice, and the free catalog [`gradium/phonon-voices`](https://huggingface.co/gradium/phonon-voices) (Apache-2.0) in the languages the model speaks: 15 French voices, with gender, age and a short description. A catalog voice is downloaded the first time you choose it (about 256 KB). Offline, only the voices already on the disk are listed. If the chosen voice is not usable with the active model, speech falls back to Default and the screen says so.

### Testing

**Test** has a sentence field and a **Play** button. It plays the speech in your browser and shows the time to first sound. On the hub's processor the first sound should come in about a second; the engine uses half of the cores by default so the rest of the house stays responsive (set `RAYON_NUM_THREADS` in the hub's environment to change that).

### The speech address

For other clients, such as the future satellites, speech is two HTTP calls: a `POST` with the text, then a `GET` of the address it returns. The audio is a streamed WAV (24 kHz, 16-bit, mono) that starts playing while the hub is still producing it. The address is valid for 60 seconds. See the [HTTP API](../reference/http-api.md#speech).

## Not yet

Speech in (the microphone), spoken Assist replies, English sentences, shutters, brightness, colours, scenes, moods, the alarm and turning the heating off are not handled. The [HTTP API](../reference/http-api.md#assist) has the three routes the page uses.
