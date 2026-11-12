---
title: Assistant
sidebar_position: 19
description: Short French or English sentences typed on the Assistant page switch lights, open shutters, turn TVs and plugs on and off, control the music and the speakers' volume, set heating setpoints and read temperatures, with safety checks, a Simulation switch and a Wrong button.
---

The **Assistant** understands short French or English sentences about the house and acts on them or answers, with a one-line reply. Open it from **Assistant** in the sidebar, or from the round button in the bottom-right corner of every page.

It runs in the hub with fixed rules, no language model: the same sentence always gives the same result, instantly, and nothing leaves the house. It is deliberately limited. When it is not sure, it asks or does nothing; it never guesses.

## Languages

The Assistant page reads sentences and writes replies in the language of the screen: French when the web UI is in French, English when it is in English (switch it from the language toggle). The sentences understood are the same in both; the examples below are in French, and the English ones read as you would expect:

| English | French |
|---|---|
| "turn on the kitchen lights", "turn off the lamp in the office", "turn everything off" | « allume la lumière de la cuisine », « éteins la lampe du bureau », « éteins tout » |
| "turn the living room lights red", "turn on the lamp in blue" | « passe le salon en rouge », « allume la lampe en bleu » |
| "set the bedroom to 19.5", "turn up the heating in the office", "turn the heat down a bit" | « règle la chambre à 19,5 », « monte le chauffage du bureau », « baisse un peu le chauffage » |
| "what's the temperature in the kitchen?", "how cold is it outside?" | « quelle température dans la cuisine ? », « il fait combien dehors ? » |
| "open the shutters to 30%", "close all the blinds" | « ouvre les volets à 30 % », « ferme tous les volets » |
| "turn off the TV", "switch on the coffee plug" | « éteins la télé », « allume la prise café » |
| "play Shape of You", "pause", "next song", "turn the volume up", "mute the kitchen", "what's playing?" | « joue Shape of You », « pause », « chanson suivante », « monte le son », « coupe le son de la cuisine », « qu'est-ce qu'on écoute ? » |

In English a colour is said after "in" or "to", or as the last word ("turn the lamp red"); before the light word it is part of a name ("the red lamp"). Numbers can be digits or words ("nineteen point five"). Answer a question with "yes", "no", "cancel" or a room's name. Satellites still speak French.

## What it understands

Rooms are your own rooms, by name ("salon", "la chambre de Léa", "chez Léa", "Léa"). Accents, capitals and a one-letter typo in a long word don't matter. A room named after its person ("Léa") is called "la chambre de Léa" in the replies, the room buttons and the examples.

**Lights**, on or off, for a room or for named lights in it:

- « allume la lumière du salon », « éteins le salon », « éteins tout dans le bureau »
- « éteins la led du salon », « allume la lampe de la chambre »

A room's lights are the ones a mood's "lights" action uses: every light of the room, the ones hidden from the dashboard included, but not groups. Lights already in the wanted state are left alone; an offline light gets no command and the reply says it did not answer.

**Light colours**, said after "en", switch the lights on in that colour:

- « allume la lumière du salon en rouge », « mets la led en bleu », « passe le salon en vert »

The colours are blanc, rouge, orange, jaune, vert, cyan (turquoise), bleu, violet (mauve), magenta and rose, in any gender or number. Only lights that take a colour get one; the others in the room are left as they are and the reply names them. A colour word without "en" is part of a light's name (« allume la lampe rose »), and a colour with an off order, heating, shutters or music is not understood.

**Light brightness**, said after "à" with a percentage, switches the lights on at that level, with or without a colour:

- « allume la led du salon à 50 % », « allume les leds du salon en bleu à 100 % », « mets la lampe à 30 % en rouge »; in English "turn the lamp on at 50 %", "set the lamp to 30 %"

The percentage goes from 1 to 100; outside that the assistant answers with the range. Only lights that can be dimmed get it; the others in the room are left as they are and the reply names them. "Mets le salon à 20 %" stays not understood (it could be the heating), as does a percentage with an off order, plugs, TVs or heating.

**Heating setpoints**, for a room whose [climate](climate.md) zone is active:

- an exact value: « mets le salon à 20 », « règle la chambre de Léa à 19,5 degrés »
- a change from the current setpoint: « monte le chauffage d'un degré au bureau », « baisse un peu le chauffage du salon » (a bare "monte" or "baisse" is 1 °C, "un peu" is 0,5 °C), « chauffage +2 au bureau »

A change starts from what the room's devices are really set to, a temperature set by hand included. If they disagree (two units, one changed by hand), the assistant names the values and asks for a temperature. When the room's current reading is fresh and already past the setpoint, a raise starts from the reading if it is higher, and a lower from the reading if it is lower: with a setpoint of 15,5 °C in a room at 18 °C, "monte" gives 19 °C.

The setpoint holds until the zone's next scheduled change, at most the zone's manual hold (4 h by default): the same hold a change made by hand on the device gets. It replaces a temperature set by hand on a unit of the zone, so every unit follows it. "Allume le chauffage" alone is not a setpoint; the assistant asks for a temperature.

**Shutters**, open, closed or at a position, for a room, a named shutter or the whole house:

| Sentence | Does |
|---|---|
| « ouvre les volets du salon » | opens every shutter in the Salon |
| « ferme les volets du salon », « baisse les stores du salon » | closes them |
| « ouvre les volets du salon à 30 % » | moves them to 30 % open |
| « ferme le volet de la baie » | the shutter whose name holds "baie" |
| « ferme tous les volets », « ouvre tous les volets » | every shutter in the house, no question |
| « ferme les volets » | asks "Dans quelle pièce ?" with the rooms that have shutters (none asked when only one has any) |

"Volet", "store" and "volet roulant" all work; "monte" / "remonte" open and "baisse" / "descends" close. A position is the percentage open, 0 closed and 100 open, as the shutter's card shows it, and goes with an open verb only: « ferme les volets à 30 % » is ambiguous, so the assistant answers with an example instead of moving anything. A shutter that only opens and closes is named in the reply as unable to take a position; the others still move. Shutters already where they are asked to be are left alone, an offline one gets no command and the reply says it did not answer. The reply comes as soon as each shutter starts moving.

**TVs and plugs**, on or off, for a room, a named one or the whole house:

| Sentence | Does |
|---|---|
| « allume la télé », « éteins la télé du salon » | turns the TV on or off |
| « éteins toutes les télés » | every TV in the house |
| « éteins la prise du bureau », « allume la prise bureau » | the plugs of the Bureau |
| « éteins toutes les prises » | every plug in the house, no question |
| « allume les 2 prises du bureau » | the plugs of the Bureau |

"Télé", "télévision" and "TV" are TVs, "prise" is a plug. Only a switch whose **Show as** type is **TV** or **Plug** is used: set it once in the device's settings. A plain switch, a siren, a door opener, a chime, a group or a hub-made virtual switch is never touched, whatever its name. With no room and no "toutes", the assistant asks "Dans quelle pièce ?"; with several plugs in the room and a name that fits none, it asks which one. Devices already on or off as asked are left alone, an offline one gets no command and the reply says it did not answer. A sentence with a lights word still means lights: « éteins le salon » never turns off a TV or a plug.

**Music**, on a [media player](media-players.md):

| Sentence | Does | Reply |
|---|---|---|
| « mets pause », « pause la musique », « stop la musique » | pauses | "Musique en pause." |
| « reprends la musique », « relance la musique » | resumes | "Je reprends Get Lucky." |
| « mets de la musique », « lance de la musique » | plays; an idle player picks what | "Je joue Get Lucky." |
| « chanson suivante », « passe à la suivante » | next song | "Get Lucky, Daft Punk." |
| « chanson précédente », « reviens en arrière » | previous song | the song's title and artist |
| « c'est quoi cette chanson ? », « qu'est-ce qu'on écoute ? » | nothing, reads the player | "Get Lucky, de Daft Punk." or "Rien ne joue." |
| « joue Daft Punk », « joue la vie en rose » | searches and plays, the words as you said them | "Je joue …" with the song found |

The reply names the song the player reports once it has done it. **Play by name is "joue …" only**: « mets Daft Punk » is not a search, so "mets" keeps meaning what it means for the heating, and « mets de la musique classique » answers with a tip to say "joue". A music sentence needs a music word ("musique", "chanson", "morceau", "titre", "pause", "suivante", "précédente"): a bare "passe" or "mets" never touches the music. A player that cannot search says so, and nothing is sent.

With one player in the house, it is the one. With several, a sentence heard by a [satellite](talk-to-a-satellite.md) uses the player in that satellite's room; typed on the page, or with no player in that room, the assistant asks "Quel lecteur ?". An offline player answers "Le lecteur ne répond pas." and one that does not confirm in time "Le lecteur n'a pas répondu.".

**Speaker volume**, on the [sound satellites](sound.md):

| Sentence | Does |
|---|---|
| « monte le son », « baisse le son » | the Music volume ± 10 points |
| « baisse un peu le son » | ± 5 points |
| « monte le son de 20 », « mets le son à 60 % » | ± 20 points, or 60 % |
| « coupe le son », « remets le son » | Mute on or off |
| « baisse le son du salon » | the Salon's satellites, wherever it was said |

Without a room, a sentence heard by a satellite changes that satellite's room; typed on the page, the assistant asks "Dans quelle pièce ?" among the rooms with a satellite. Only the Music volume changes, never the Speech volume.

**Temperatures**: « quelle température fait-il dans le salon ? », « il fait combien au bureau ? », « il fait combien dehors ? ». A room's temperature is its climate zone's reading, else its thermometer; outdoors is the climate engine's outdoor sensor, else the weather. An old reading is said with its age.

## Safety

- **A statement or a question is not an order.** "Il fait 18 degrés au salon", "la lumière du salon est allumée ?" or "qui a éteint le salon ?" never act (a temperature question is answered). A polite request still is one: « peux-tu allumer la lumière du salon ? ».
- **Not sure, no action.** Two opposite orders ("allume et éteins"), a negation ("n'éteins pas"), a percentage that is not a shutter position ("mets le salon à 50 %") or a device it does not handle yet (curtains, sirens) give "Je n'ai pas compris", with a few example sentences built from your rooms.
- **Ambiguous room, a question.** "La chambre" with two bedrooms asks "Dans quelle pièce ?" with the rooms as buttons. You can also type the answer ("Léa").
- **Unusual setpoint, a confirmation.** A setpoint outside **16–23 °C**, or more than **3 °C** away from the room's current temperature, is asked first: "Passer le Salon à 25 °C ?" with **Oui** / **Non** (typing "oui" or "non" works too). A yes applies exactly the value asked about. The 16–23 °C range is set by `assist_min_c` and `assist_max_c` in the [home settings](../reference/http-api.md#home-settings).
- **Out of the device's range, a refusal.** A value the heaters cannot take is refused with their range, never clamped silently.
- A question waits **60 seconds**. Any other sentence cancels it.

Every reply names the room, what was done and the value, for example "Chambre Léa réglée à 19 °C (avant 17,5 °C)." When a light does not answer, the reply says which one.

## Simulation and Wrong

The **Simulation** switch under the field plans and replies but sends nothing to the house; replies start with "Simulation :". It is the safe way to try phrasings. The page remembers it per browser.

The panel next to the conversation, **How it understood**, shows the selected turn: the normalised words, the intent, the room and how it was found, the devices, the value and its arithmetic ("19 − 1 = 18 °C", or "mesure 18 > consigne 15,5 : 18 + 1 = 19 °C" when the reading was the start), the safety check, each action with its result, and the time taken. **Wrong** (« Faux » in French) marks a turn as wrongly understood, so it can be found in the log later; the turn then shows **Marked wrong**.

With the history database, turns are kept 90 days. Without it, the hub keeps the last 100 turns in memory and the page says history is not kept.

## Voice

The hub can turn text into speech, in each language you set up, on its own processor, with no cloud call when it speaks. Set it up in **Settings → Assistant → Voice** (« Réglages → Assistant → Voix » in French); the gear button at the top right of the Assistant page opens that Assistant group. Speech recognition for the satellites is next to it, under **Listening** (« Écoute »), and the speaker satellites under **Sound**. The Assistant page reads each new reply aloud once the hub speaks the page's language (French or English) with a model: the speaker switch in its header turns that off, remembered per browser, and **▶** on a reply reads it again. The [sound satellites](sound.md#speak-in-a-room) say sentences with it too: from Settings → Assistant → Sound, a flow's Speak card or the MCP tool `speak`.

The engine is [Phonon](https://huggingface.co/gradium) by Gradium. The hub never ships the model files: you download them yourself, under their licence.

### Licence and token

The French models, `gradium/phonon-40m-fr` and `gradium/phonon-40m-multilingual`, are **gated** on Hugging Face and licensed **CC BY-NC 4.0**: non-commercial use only. To use them:

1. On the model's page on huggingface.co, accept the terms.
2. In your Hugging Face account, open **Settings, Access Tokens** and create a token with the **Read** role.
3. Under **Add a model**, open **Hugging Face token: only needed for gated repositories** (it also says whether a token is set), paste the token and save. The field is masked and write-only: the hub never shows the token again. **Clear** removes it. Changing it does not restart the hub, and clearing it keeps the models you already installed.

The token is sent only to huggingface.co.

### Adding a model

Under **Add a model**, paste the model page address (`https://huggingface.co/owner/repo`, `hf://owner/repo` or `owner/repo`) and press **List files**. The hub shows the weight files the repository offers, with their sizes:

| File | Size (40m models) | Notes |
| --- | --- | --- |
| `model.q8.gguf` | about 75 MB | The default pick. |
| `model.q4k.gguf` | about 60 MB | Smaller. |
| `model.safetensors` | about 122 MB | Full precision. |

Choose one and press **Download**. It runs in the background with a progress bar and a **Cancel** button; the model appears in the list only when every file is complete and checked. One download runs at a time. If the repository is gated and no token is saved, or the account has not accepted the terms, the screen says so and links to the model page. A restart of the hub drops an unfinished download.

No internet on the hub, or a token you would rather not use? Under **Upload from this computer**, press **Choose a folder** and pick the model's folder (the same files, from a download made elsewhere). The hub keeps the files once the weight file and the required small files are all there; it does not load the model at that point, so a model that cannot load shows as an Error once a language uses it, and you can remove it.

### Languages, models and voices

**Languages** (« Langues ») has one tab per language the hub speaks, named in the screen's language (French, English…), each with a dot for its model's state: green Ready, amber Loading, red Error, grey No model. The first tab is the **house language**: the one speech uses when a request names no language. Any other tab has a **Make house language** button that moves it first. The screen remembers the tab you were on, per browser.

**+ Add** lists the languages the installed models speak that are not set up yet. Pick one: if a single model speaks it, it is added with that model; if several do, the screen asks which one. A model whose files name no language is offered under every language; when it is installed, **+ Add** also takes a typed language code (such as `de`). When no installed model speaks another language, it says so. So the first step is always to install a model, then to add its language.

Each language has its own model. One multilingual model can serve several languages, or each language can have its own model. Every model a language uses stays loaded in memory (about 125 MB for a 40m `q8` model), so switching from one language to another costs nothing; a model no language uses is unloaded.

In a language's tab:

- **Model** (« Modèle ») chooses among the installed models that speak this language (a model whose files list no languages is offered everywhere, marked "languages unknown"), with its state: No model, Loading, Ready or Error with a message. Changing the model resets the voice to Default. A broken or missing model never stops the hub or text Assist: speech in that language just shows an error.
- **Try it** (« Essai ») has the sentence field and **Play**. It starts with a sentence in that language (for French, "Bonjour, je suis l'assistant de la maison."); for a language without one, the field starts empty and **Play** waits until you type something. What you type is kept per language until you leave the page. It plays the speech in your browser and shows the time to first sound.
- **Voice** (« Voix ») lists **Default**, the model's own voice, and the free catalog [`gradium/phonon-voices`](https://huggingface.co/gradium/phonon-voices) (Apache-2.0) voices in this language, such as 15 French voices, with gender, age and a short description, in alphabetical order (an accented first letter sorts with its plain letter, so Élise comes before Eve). Choose one for this language; a catalog voice is downloaded the first time you choose it or listen to it (about 256 KB). The **▶** button on each voice speaks the tab's sentence in that voice without choosing it. Once the catalog list has been fetched it is kept on the disk, so the whole catalog stays listed offline (a voice not yet downloaded is marked unavailable offline); with no copy yet, no catalog voice is listed. If the chosen voice is not usable with the language's model, speech falls back to Default and the screen says so.
- **Remove language** at the bottom stops speaking it, after a confirmation.

**Installed models** lists each model with its weight file, size, short commit, licence, the languages it speaks and **Used by**: the languages that speak with it. **Remove** deletes a model after a confirmation; when languages use it, the confirmation names them, and they stay in the list with no model until you choose another one.

### Testing

On the hub's processor the first sound should come in about a second. The engines share half of the cores by default so the rest of the house stays responsive (set `RAYON_NUM_THREADS` in the hub's environment to change that).

### The speech address

For other clients, such as the future satellites, speech is two HTTP calls: a `POST` with the text (and, if you want, its language; the house language otherwise), then a `GET` of the address it returns. The audio is a streamed WAV (24 kHz, 16-bit, mono) that starts playing while the hub is still producing it. The address is valid for 60 seconds. See the [HTTP API](../reference/http-api.md#speech).

## Not yet

Speaking to the assistant works through the satellites: see [Talk to a satellite](talk-to-a-satellite.md); a spoken turn shows the satellite and its room in the history. Stopping a shutter half-way ("stop"), scenes, moods, the alarm and turning the heating off are not handled. The [HTTP API](../reference/http-api.md#assist) has the three routes the page uses.
