import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { connect } from "./lib/app.svelte";

// The stream's first event is the summary; the page draws once it is there.
connect();
mount(App, { target: document.getElementById("app")! });
