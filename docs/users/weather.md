---
title: Weather
sidebar_position: 14
description: Outdoor weather from Open-Meteo as entities and a REST document, how to turn it on, what each entity means, and how it behaves when the network is down.
---

The hub can fetch the outdoor weather for your city and publish it as a **Weather** device. Its entities behave like any other sensor: flows can trigger on them, the history records them, and you can place them in a room. The same data is served as one document at `GET /api/v1/weather` and by the `get_weather` MCP tool.

Weather is **off by default**. While it is off, the hub makes no request to anyone.

## Turn it on

Set the home location first (the city step of the setup wizard, or **Settings → Location**): the weather uses it. Then open **Settings → Weather** (or the Services step of the setup wizard), switch **Enabled** on and save. The hub restarts to pick the change up.

- **Enabled** is off by default.
- **Refresh every (minutes)** is 15 by default and must be between 10 and 1440 when weather is on.
- The city and country come from your home location: the city is required, and the country is an optional two-letter code (`FR`) that keeps the lookup to one country, so `Paris` is not Paris, Texas.
- A value the hub cannot run with (a refresh below 10, a three-letter country) is refused when you save, and nothing is changed.

With a home location set, the hub uses its coordinates and makes no lookup. A city saved without coordinates is looked up once at start, and the hub logs the place it chose (name, region, country and coordinates). If it finds nothing, it logs an error naming the city and the weather stays unavailable until you fix the location in Settings. If the lookup fails because of the network, the hub retries after 30 seconds, then waits twice as long each time, up to 15 minutes between tries. The rest of the house keeps running either way.

See the [Settings reference](../reference/configuration.md#weather) for the stored fields, and `get_settings` / `set_settings` in [MCP tools](../reference/mcp-tools.md) to change them from an assistant.

## In the web UI

With weather on, the house overview has a weather card in its header, next to the greeting: the condition, the temperature, the feels-like temperature, today's low and high, and the sunset time. The card hides when there is no data or the last good data is more than six hours old. The header's detail line also mentions rain expected within the hour and frost tonight.

The weather document also carries the next five days, starting tomorrow. Each day has its condition, lowest and highest temperature, highest feels-like temperature, precipitation total and chance, wind speed, gusts and direction, UV index, and the day's mean humidity, pressure and cloud cover.

Tap the card to open the weather page (`/weather`), from the same data as the document above. It opens on **Now**: a big temperature with the condition and the feels-like temperature, and on the right today's low and high, the lowest temperature tonight and whether rain is expected within the hour. Under it are tiles for **Wind** (speed, an arrow showing where it blows to, the compass point and gusts), **Humidity**, **Pressure**, **Clouds**, **Rain** (today's total and chance), **Max gusts**, **UV index** (with its level) and **Sun** (sunset and sunrise).

Under the tiles, a **Forecast** strip shows **Today** and the next five days, each with its condition, high, low and chance of rain. Tap a day to see that day on the same page, with the tiles showing its values (the address becomes `/weather?day=<date>`); tap **Today** to come back. The footer gives the place the hub found, the update time and the Open-Meteo credit, plus "Data from HH:MM" when the data is stale. With weather off, the page says the weather is unavailable and to turn it on in **Settings → Weather**. The header rules and the page layout are in [The web UI](web-ui.md#home-header).

## The entities

All units are metric. Entity ids are `weather_<key>`, for instance `weather_wind_gusts`. The hub names them in English ("Wind gusts", device "Weather"); the web shows them in its language ("Rafales", device "Météo" in French) and finds them by either name, until you rename one.

| Key | Meaning |
| --- | --- |
| `temperature` | Air temperature now, °C |
| `apparent_temperature` | Feels-like temperature, °C |
| `humidity` | Relative humidity, % |
| `condition` | `clear`, `partly_cloudy`, `cloudy`, `fog`, `drizzle`, `rain`, `freezing_rain`, `snow`, `showers` or `thunderstorm` |
| `daylight` | On while the sun is up |
| `cloud_cover` | Cloud cover, % |
| `precipitation` | Precipitation over the current interval, mm |
| `wind_speed` | Mean wind, km/h |
| `wind_gusts` | Gusts, km/h |
| `wind_direction` | The direction the wind comes from, degrees |
| `pressure` | Sea-level pressure, hPa |
| `uv_index` | UV index |
| `today_min`, `today_max` | Today's lowest and highest temperature, °C |
| `today_precipitation` | Today's precipitation total, mm |
| `today_precipitation_probability` | Today's highest chance of precipitation, % |
| `today_wind_gusts_max` | Today's strongest gust, km/h |
| `rain_next_hour` | Precipitation expected in the next hour, mm |
| `tonight_min` | The lowest temperature from now until 09:00 the next morning, °C (frost watch) |
| `tomorrow_condition` | Tomorrow's condition, one of the `condition` values |
| `tomorrow_min`, `tomorrow_max` | Tomorrow's lowest and highest temperature, °C |
| `tomorrow_precipitation` | Tomorrow's precipitation total, mm |
| `tomorrow_precipitation_probability` | Tomorrow's highest chance of precipitation, % |
| `tomorrow_wind_gusts_max` | Tomorrow's strongest gust, km/h |
| `tomorrow_uv_index_max` | Tomorrow's highest UV index |

The Weather device has no room. Move an entity to a room with the usual override if you want it there. A value Open-Meteo does not provide reads as unknown.

## Freshness and offline

- The hub fetches every `refresh_minutes`, with about 10 % random jitter. A failed fetch is logged once per outage and the last good values stay.
- After a failed fetch the hub tries again sooner: 1 minute later, then 2, 4, 8… minutes, never longer than `refresh_minutes`. The first success goes back to the regular period.
- Every success is also kept on the broker (retained on `<prefix>/weather/cache`). After a restart the hub serves that last document at once, so the header and the Weather page have data even if Open-Meteo is down at that moment; `stale` still follows the real fetch time.
- When the last success is older than twice `refresh_minutes`, `stale` is `true` in `GET /api/v1/weather`.
- After one hour without a success (counting the document kept from before a restart), whatever `refresh_minutes` is, the weather entities go **unavailable**, so a flow sees "unavailable" and not an old number. `GET /api/v1/weather` keeps serving the last document, with `stale` set by the rule above.
- The entities come back as soon as a fetch succeeds. After a restart they read unavailable until the first fetch, or until the first failed one if the kept document is under an hour old.
- Turning weather off removes the Weather device: on the next start the hub clears what it left on the broker. History rows are kept.

## Tell me when Open-Meteo is unreachable

Turn on **Notify me when unreachable** in the Weather settings to get a push alert when the hub has had no answer from Open-Meteo for 30 minutes in a row. You get one alert when it goes down (with the last reason) and one when it answers again, saying how long it was away. A short blip under 30 minutes stays silent, and nothing repeats while the outage lasts. The count starts again when the hub restarts. The switch is off by default.

## Flow ideas

These are built in the flow editor, or with `set_flow` through [MCP](assistants-mcp.md). Use the ids from your own house.

- **Close the awning in the wind:** trigger on `weather_wind_gusts`, check it is above 50, then close the awning cover.
- **Rain is coming, a window is open:** trigger on `weather_rain_next_hour`, check it is above 0.2, check the window sensor is open, then send a notification.
- **Frost tonight:** a schedule trigger at 18:00, check `weather_tonight_min` is below 1, then notify.
- **Hot day:** check `weather_today_max` is above 30 at 09:00, then close the shutters.

See [Flows](flows.md) for the nodes.

## Privacy

While weather is enabled, the hub sends the **city name** (and the country, if set) to Open-Meteo's geocoding service, then the **coordinates of the place it found** with each forecast request. Open-Meteo sees your public IP address, as any web service does. Nothing else leaves the house: no device names, no entity states. While weather is disabled, nothing is sent.

## Credit

Weather data is provided by [Open-Meteo.com](https://open-meteo.com/) and licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Open-Meteo's free service is meant for light, non-commercial use; the default 15-minute refresh stays far below its limits.
