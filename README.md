# Raspberry Pi 3 info screen
This is a personal setup that I use home with Raspberry Pi 3 to get some info we need on a day-to-day basis


### Prerequisites
Google Calendar authentication cannot be handled purely by a Dockerfile.
You’ll need a Google Cloud project, enable the Calendar API,
create a browser OAuth client ID, and add the dashboard origin such as http://localhost:8080 to the authorized JavaScript origins.
Google’s browser flow uses a client ID and API key; a client secret is not used for browser applications. (developers.google.com)

## First time google calendar setup
For Google Calendar:
1. Create a Google Cloud project.
2. Enable the Google Calendar API.
3. Create an API key.
4. Create an OAuth client for a web application.
5. Add http://localhost:8080 as an authorized JavaScript origin.
6. Replace YOUR_GOOGLE_CLIENT_ID and YOUR_GOOGLE_API_KEY.
7. Click Authorize Calendar once on the display.

The first authorization will require interaction.
Depending on browser token persistence, the Pi may require reauthorization after a restart or token expiration.
Google recommends the read-only Calendar scope for displaying events. (developers.google.com)

# Build and run dockerimage
``` bash
docker build -t info-screen .
docker run -d \
--name info-screen \
--restart unless-stopped \
-p 8080:80 \
info-screen
```

Avaiable here at
http://localhost:8080


# Launch in fullscreen kiosk mode on the Raspberry Pi 3
``` bash
chromium \
--kiosk \
--noerrdialogs \
--disable-infobars \
--disable-session-crashed-bubble \
http://localhost:8080
```