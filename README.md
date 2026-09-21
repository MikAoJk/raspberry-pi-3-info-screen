# Raspberry Pi 3 info screen
This is a personal setup that I use home with Raspberry Pi 3 to get some info we need on a day-to-day basis

### Prerequisites
Make sure you have the rust installed using this command:
#### Rust
```bash script
rustc --version
```

#### Cargo
Make sure you have cargo installed using this command:
```bash script
cargo --version
```

### Build
Build the code without running it
```bash script
cargo build
```

### Environment
Set the required environment variables before running the app:

```bash
export GOOGLE_CLIENT_ID="your-client-id.apps.googleusercontent.com"
export GOOGLE_CLIENT_SECRET="your-google-client-secret"
```

Optional values:

```bash
export WEATHER_LATITUDE="58.14574000943632"
export WEATHER_LONGITUDE="8.06137337891726"
export CALENDAR_ID="your-calendar-id@group.calendar.google.com"
export GOOGLE_REDIRECT_URI="http://localhost:8080/oauth/callback"

```

### Run
Run the code
```bash script
cargo run
```

#### Running the application locally

#####  Create a docker image of an app
Creating a docker image should be as simple as
``` bash
docker build -t infoscreeen .
```

#### Running a docker image
``` bash
docker run -d \
--name infoscreeen \
--restart unless-stopped \
-p 8080:8080 \
infoscreeen
```
Avaiable here at
http://localhost:8080


# Launch in fullscreen kiosk mode on the Raspberry Pi 3
``` bash
firefox \
--kiosk \
http://localhost:8080
```

### Example output from my calendar
![screenshot.png](screenshot.png)
