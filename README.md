# Raspberry Pi 3 info screen
This is a personal setup that I use home with Raspberry Pi 3 to get some info we need on a day-to-day basis

# Build and run dockerimage
``` bash
docker build -t info-screen .
docker rm -f info-screen 2>/dev/null || true
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

# Prerequisites onetime setup
For Google Calendar integration:
1. Create a Google Cloud project.
2. Enable the Google Calendar API.
3. Create an OAuth client for a web application.
4. Add http://localhost:8080 as an authorized JavaScript origin.
5. Replace CLIENT_ID.
6. Click Authorize Calendar once on the display.