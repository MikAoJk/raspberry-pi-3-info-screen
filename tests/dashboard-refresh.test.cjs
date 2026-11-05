const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const html = fs.readFileSync(path.join(__dirname, "../static/index.html"), "utf8");
const script = html.split("<script>")[1].split("</script>")[0];
const refreshScript = script.slice(script.indexOf("    let googleAuthAttempted"));

function createDashboard(fetch) {
    const renders = [];
    const errors = [];
    const timers = [];
    const context = vm.createContext({
        fetch,
        renderWeather: (data, status) => renders.push({ section: "weather", data, status }),
        renderCalendar: (data, status) => renders.push({ section: "calendar", data, status }),
        console: { error: (...args) => errors.push(args) },
        startGoogleAuth: async () => {},
        startClockSlideshow: async () => {},
        updateClock() {},
        fitCalendarEvents() {},
        cancelAnimationFrame() {},
        requestAnimationFrame() {},
        ResizeObserver: class { observe() {} },
        document: { getElementById: () => ({}) },
        setInterval: (callback, delay) => timers.push({ callback, delay }),
    });
    vm.runInContext(refreshScript, context);
    return {
        renders,
        errors,
        refresh: timers.find(({ delay }) => delay === 60000).callback,
    };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));
const snapshot = {
    weather: { description: "Sunny" },
    weather_status: "fresh",
    calendar: { days: [] },
    calendar_status: "fresh",
};
const success = (data = snapshot) => ({ ok: true, json: async () => data });
const failures = {
    network: async () => { throw new Error("Offline"); },
    http: async () => ({ ok: false, status: 503 }),
    json: async () => ({ ok: true, json: async () => { throw new SyntaxError("Invalid JSON"); } }),
};

for (const [name, failure] of Object.entries(failures)) {
    test(`${name} failure on initial load marks both sections unavailable`, async () => {
        const dashboard = createDashboard(failure);
        await settle();
        assert.deepEqual(dashboard.renders, [
            { section: "weather", data: undefined, status: "stale" },
            { section: "calendar", data: undefined, status: "stale" },
        ]);
        assert.equal(dashboard.errors.length, 1);
    });

    test(`${name} periodic failures retain cached data and recovery clears warnings`, async () => {
        let fetchResponse = async () => success();
        const dashboard = createDashboard(() => fetchResponse());
        await settle();

        fetchResponse = failure;
        for (let attempt = 0; attempt < 2; attempt++) {
            await dashboard.refresh();
            assert.deepEqual(dashboard.renders.slice(-2), [
                { section: "weather", data: snapshot.weather, status: "stale" },
                { section: "calendar", data: snapshot.calendar, status: "stale" },
            ]);
        }
        assert.equal(dashboard.errors.length, 2);

        const updated = { ...snapshot, weather: { description: "Rain" } };
        fetchResponse = async () => success(updated);
        await dashboard.refresh();
        assert.deepEqual(dashboard.renders.slice(-2), [
            { section: "weather", data: updated.weather, status: "fresh" },
            { section: "calendar", data: updated.calendar, status: "fresh" },
        ]);
    });
}

test("stale renderers display warnings without discarding available data", () => {
    const element = () => ({
        textContent: "",
        children: [],
        set innerHTML(value) { this.children = []; this.textContent = value; },
        appendChild(child) { this.children.push(child); },
    });
    const elements = Object.fromEntries(
        ["weather", "weather-icon", "hourly-weather", "events"].map((id) => [id, element()]),
    );
    const context = vm.createContext({
        document: {
            getElementById: (id) => elements[id],
            createElement: element,
            querySelectorAll: () => [],
        },
        window: { matchMedia: () => ({ matches: false }) },
    });
    vm.runInContext(
        script.slice(script.indexOf("    function renderWeather"), script.indexOf("    async function startGoogleAuth")),
        context,
    );

    context.renderWeather(snapshot.weather, "stale");
    context.renderCalendar(snapshot.calendar, "stale");
    assert.equal(elements.weather.textContent, "Sunny (cached; update failed)");
    assert.equal(elements.events.children[0].textContent, "Showing cached calendar; update failed.");
    assert.equal(elements.events.children[1].className, "week-grid");

    context.renderWeather(undefined, "stale");
    context.renderCalendar(undefined, "stale");
    assert.equal(elements.weather.textContent, "Weather unavailable.");
    assert.equal(elements.events.textContent, "Calendar unavailable.");

    context.renderWeather(snapshot.weather, "fresh");
    context.renderCalendar(snapshot.calendar, "fresh");
    assert.equal(elements.weather.textContent, "Sunny");
    assert.equal(elements.events.children.length, 1);
    assert.equal(elements.events.children[0].className, "week-grid");
});
