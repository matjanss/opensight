//! A protocol v3 test client: connects to a bonded sensor, reads and subscribes to
//! everything, optionally exercises the writes, and writes the event log as the app does.
//! For bench tests over BLE without the UI (MEASUREMENT step 5).
//!
//! cargo run --example probe -- <seconds> <log file> [writes | drive]
//!
//! `writes`: after connecting, rewrite the settings unchanged, send an invalid settings
//! write (span 10 mm), write the calibration record back with a date, and run the Control
//! commands (calibration mode on, a template point, no point, off), each checked against the
//! Status notifications. Prints every shot, every event's verdict, and a summary.
//!
//! `drive`: the calibration wizard and the field check (src/calwizard.rs), in test mode (each
//! spark at the point it carries: the simulator's), run by commands on stdin, one per line:
//! `check` (the field check: prints `READY check`, then `CHECK <sparks> <mean> <worst>
//! PASS|FAIL`), `calibrate` (prints `READY calibrate`, collects the template's sparks, fits,
//! writes, prints the report and `CALIBRATED <id>` once the sensor confirms it), `quit`.
//! A script on the serial port makes the sparks after each READY.

#[path = "../src/calwizard.rs"]
#[allow(dead_code)]
mod calwizard;
#[path = "../src/eventlog.rs"]
#[allow(dead_code)]
mod eventlog;
#[path = "../src/protocol.rs"]
#[allow(dead_code)]
mod protocol;

use std::time::{Duration, Instant};

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use protocol as proto;

async fn find_sensor(adapter: &btleplug::platform::Adapter) -> Result<Peripheral, String> {
    adapter
        .start_scan(ScanFilter {
            services: vec![proto::SERVICE_UUID],
        })
        .await
        .map_err(|e| e.to_string())?;
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        for p in adapter.peripherals().await.map_err(|e| e.to_string())? {
            if let Ok(Some(props)) = p.properties().await
                && (props.services.contains(&proto::SERVICE_UUID)
                    || proto::decode_adv(&props.manufacturer_data).is_some())
            {
                println!(
                    "probe: found {:?} ({}), adv {:?}",
                    props.local_name,
                    p.id(),
                    proto::decode_adv(&props.manufacturer_data)
                );
                adapter.stop_scan().await.ok();
                return Ok(p);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err("no sensor advertising within 30 s".into())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let log_path = std::path::PathBuf::from(args.get(2).cloned().unwrap_or("probe.log".into()));
    let writes = args.get(3).is_some_and(|s| s == "writes");
    let drive = args.get(3).is_some_and(|s| s == "drive");
    let _ = std::fs::remove_file(&log_path);
    let mut log_lines: Vec<String> = Vec::new();

    let manager = Manager::new().await.map_err(|e| e.to_string())?;
    let adapter = manager
        .adapters()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or("no adapter")?;
    let p = find_sensor(&adapter).await?;
    p.connect().await.map_err(|e| format!("connect: {e}"))?;
    p.discover_services()
        .await
        .map_err(|e| format!("discover: {e}"))?;
    // After a firmware update BlueZ first answers from its cached layout; the sensor's
    // "services changed" indication makes it discover again. The sensor sends that only on
    // an encrypted link, so read the Version (in every layout, and it needs encryption) first.
    // Subscribe to "services changed" (0x2A05) ourselves, before the link is secured, so the
    // sensor's indication has a live subscription (BlueZ handles it and discovers again).
    let sc_uuid = uuid::Uuid::from_u128(0x00002a05_0000_1000_8000_00805f9b34fb);
    if let Some(sc) = p.characteristics().iter().find(|c| c.uuid == sc_uuid) {
        println!(
            "probe: subscribe to services changed: {:?}",
            p.subscribe(sc).await.map_err(|e| e.to_string())
        );
    }
    if let Some(v) = p.characteristics().iter().find(|c| c.uuid == proto::VERSION_UUID) {
        println!(
            "probe: version read (encrypts the link): {:?}",
            p.read(v).await.map_err(|e| e.to_string())
        );
    }
    for attempt in 1..=10 {
        if p.characteristics().iter().any(|c| c.uuid == proto::EVENT_UUID) {
            break;
        }
        println!(
            "probe: the v3 characteristics aren't there yet (cached layout?), discovering again ({attempt})"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        p.discover_services()
            .await
            .map_err(|e| format!("discover: {e}"))?;
    }
    let chars = p.characteristics();
    let find = |u| {
        chars
            .iter()
            .find(|c| c.uuid == u)
            .cloned()
            .ok_or(format!("missing {u}"))
    };
    let (version, shot, event, status, control, calibration, settings) = (
        find(proto::VERSION_UUID)?,
        find(proto::SHOT_UUID)?,
        find(proto::EVENT_UUID)?,
        find(proto::STATUS_UUID)?,
        find(proto::CONTROL_UUID)?,
        find(proto::CALIBRATION_UUID)?,
        find(proto::SETTINGS_UUID)?,
    );
    let v = p.read(&version).await.map_err(|e| format!("read version: {e}"))?;
    println!("probe: protocol version {v:?}");
    if v != [proto::PROTOCOL_VERSION] {
        return Err("not version 3".into());
    }
    let read_config = async |p: &Peripheral| -> Result<(proto::Calibration, proto::Settings), String> {
        let cal = proto::decode_calibration(&p.read(&calibration).await.map_err(|e| e.to_string())?)
            .ok_or("calibration decode")?;
        let set = proto::decode_settings(&p.read(&settings).await.map_err(|e| e.to_string())?)
            .ok_or("settings decode")?;
        Ok((cal, set))
    };
    let st0 =
        proto::decode_status(&p.read(&status).await.map_err(|e| e.to_string())?).ok_or("status decode")?;
    let (cal, set) = read_config(&p).await?;
    let (cfg_id, cfg_line) = eventlog::cfg_line(&cal, &set);
    println!("probe: status {st0:?}");
    println!("probe: settings {set:?}");
    println!("probe: {cfg_line}");
    println!(
        "probe: cfg id from the reads {cfg_id:08X}, Status says {:08X}: {}",
        st0.cfg,
        if cfg_id == st0.cfg { "MATCH" } else { "DIFFERENT" }
    );
    log_lines.push(cfg_line);
    let mut cfg_seen = std::collections::HashSet::from([cfg_id]);

    let mut stream = p.notifications().await.map_err(|e| e.to_string())?;
    for c in [&status, &shot, &event] {
        p.subscribe(c).await.map_err(|e| format!("subscribe: {e}"))?;
    }
    // PROBE_FROM=<boot hex>:<seq>: ask for hits from there (an app that has that boot's
    // hits up to seq - 1, the shot store test, MEASUREMENT 5e); otherwise (0, 0): an app with none.
    let (from_boot, from_seq) = std::env::var("PROBE_FROM")
        .ok()
        .and_then(|v| {
            let (b, q) = v.split_once(':')?;
            Some((u32::from_str_radix(b, 16).ok()?, q.parse().ok()?))
        })
        .unwrap_or((0, 0));
    println!("probe: shot request from boot {from_boot:08X} seq {from_seq}");
    p.write(&shot, &proto::encode_shot_request(from_boot, from_seq), WriteType::WithResponse)
        .await
        .map_err(|e| e.to_string())?;
    p.write(
        &event,
        &proto::encode_event_request(0, 0),
        WriteType::WithResponse,
    )
    .await
    .map_err(|e| e.to_string())?;

    if writes {
        let w = async |c: &btleplug::api::Characteristic, v: &[u8], what: &str| match p
            .write(c, v, WriteType::WithResponse)
            .await
        {
            Ok(()) => println!("probe: wrote {what} ({} bytes)", v.len()),
            Err(e) => println!("probe: write {what} FAILED: {e}"),
        };
        w(&settings, &proto::encode_settings(&set), "settings unchanged").await;
        let mut bad = set.clone();
        bad.span_mm = 10;
        w(
            &settings,
            &proto::encode_settings(&bad),
            "settings with span 10 mm (must be rejected)",
        )
        .await;
        let mut rec = cal.clone();
        rec.date = 20261123;
        rec.res_ns = 12.5;
        rec.points = 7;
        rec.sparks = 56;
        w(
            &calibration,
            &proto::encode_calibration(&rec),
            "the calibration record (213 bytes, a long write)",
        )
        .await;
        w(&control, &proto::encode_cal_mode(true), "calibration mode on").await;
        w(
            &control,
            &proto::encode_point(12.5, -7.25),
            "template point (12.5, -7.25)",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let st = proto::decode_status(&p.read(&status).await.map_err(|e| e.to_string())?).unwrap();
        println!(
            "probe: status after: cal mode {}, point {} ({}, {}), cal {:08X} stale {}",
            st.flags & proto::ST_CAL_MODE != 0,
            st.flags & proto::ST_POINT != 0,
            st.pt_x,
            st.pt_y,
            st.cal,
            st.flags & proto::ST_CAL_STALE != 0
        );
        let (cal2, set2) = read_config(&p).await?;
        println!(
            "probe: read back: calibration date {} points {} sparks {} res {} (channels {}); settings {}",
            cal2.date,
            cal2.points,
            cal2.sparks,
            cal2.res_ns,
            if cal2.ch == cal.ch { "unchanged" } else { "CHANGED" },
            if set2 == set { "unchanged" } else { "CHANGED" }
        );
        println!(
            "probe: calibration id computed {:08X}",
            eventlog::calibration_id(&cal2, &set2)
        );
        w(&control, &proto::encode_no_point(), "no template point").await;
        w(&control, &proto::encode_cal_mode(false), "calibration mode off").await;
        w(&control, &[0x07], "an unknown command (must be ignored)").await;
    }

    if drive {
        // Commands from stdin, through a thread (tokio here has no stdin support).
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                if tx.send(line.trim().to_owned()).is_err() {
                    break;
                }
            }
        });
        let send = async |p: &Peripheral, actions: Vec<calwizard::Action>| {
            for a in actions {
                let (chr, bytes, what) = match &a {
                    calwizard::Action::CalMode(on) => {
                        (&control, proto::encode_cal_mode(*on).to_vec(), "calibration mode")
                    }
                    calwizard::Action::Point(x, y) => {
                        (&control, proto::encode_point(*x, *y).to_vec(), "template point")
                    }
                    calwizard::Action::NoPoint => (&control, proto::encode_no_point().to_vec(), "no point"),
                    calwizard::Action::WriteCalibration(c) => (
                        &calibration,
                        proto::encode_calibration(c).to_vec(),
                        "the calibration record",
                    ),
                };
                match p.write(chr, &bytes, WriteType::WithResponse).await {
                    Ok(()) => println!("probe: sent {what}: {a:?}"),
                    Err(e) => println!("probe: sending {what} FAILED: {e}"),
                }
            }
        };
        let mut status = st0.clone();
        let mut check: Option<calwizard::FieldCheck> = None;
        let mut wizard: Option<calwizard::Wizard> = None;
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut events_seen = 0u32;
        while Instant::now() < deadline {
            tokio::select! {
                cmd = rx.recv() => match cmd.as_deref() {
                    Some("check") => {
                        let mut c = calwizard::FieldCheck::new(&calwizard::grid_template());
                        send(&p, c.take_actions()).await;
                        check = Some(c);
                        println!("READY check");
                    }
                    Some("calibrate") => {
                        let (cal, _) = read_config(&p).await?;
                        status = proto::decode_status(&p.read(&status_chr_ref(&chars)).await.map_err(|e| e.to_string())?).unwrap_or(status);
                        match calwizard::Wizard::new(calwizard::grid_template(), &status, &cal, true) {
                            Ok(mut w) => {
                                w.start();
                                send(&p, w.take_actions()).await;
                                wizard = Some(w);
                                println!("READY calibrate");
                            }
                            Err(e) => println!("CALIBRATE FAILED {e}"),
                        }
                    }
                    Some("quit") | None => break,
                    Some(other) => println!("probe: unknown command {other:?}"),
                },
                n = tokio::time::timeout(Duration::from_millis(500), stream.next()) => {
                    let Ok(Some(n)) = n else { continue };
                    if n.uuid == proto::EVENT_UUID {
                        let e = proto::decode_event(&n.value).ok_or("event decode")?;
                        events_seen += 1;
                        if !cfg_seen.contains(&e.cfg) {
                            let (c, s) = read_config(&p).await?;
                            let (id, line) = eventlog::cfg_line(&c, &s);
                            cfg_seen.insert(id);
                            log_lines.push(line);
                        }
                        log_lines.push(eventlog::event_line(&e));
                        if let Some(c) = &mut check {
                            c.on_event(&e);
                            if c.done {
                                let r = c.result().unwrap();
                                println!("CHECK {} {:.4} {:.4} {}", r.sparks, r.mean_mm, r.worst_mm, if r.pass { "PASS" } else { "FAIL" });
                                send(&p, c.take_actions()).await;
                                check = None;
                            }
                        }
                        if let Some(w) = &mut wizard {
                            w.on_event(&e);
                            if w.point_done() {
                                let (have, want) = w.progress();
                                println!("probe: {have} of {want} sparks (ignored {}): fitting", w.ignored);
                                w.next_point();
                                match &w.phase {
                                    calwizard::Phase::Fitted(_) => {
                                        println!("{}", w.report("the bench NodeMCU").unwrap());
                                        w.write(20261123);
                                        send(&p, w.take_actions()).await;
                                    }
                                    other => println!("CALIBRATE FAILED {other:?}"),
                                }
                            }
                        }
                    } else if n.uuid == proto::STATUS_UUID
                        && let Some(st) = proto::decode_status(&n.value)
                    {
                        {
                            status = st.clone();
                            if let Some(w) = &mut wizard {
                                w.on_status(&st);
                                if let calwizard::Phase::Done { id } = w.phase {
                                    send(&p, w.take_actions()).await;
                                    println!("CALIBRATED {id:08X}");
                                    wizard = None;
                                }
                            }
                        }
                    }
                }
            }
        }
        std::fs::write(&log_path, log_lines.join("\n") + "\n").map_err(|e| e.to_string())?;
        println!(
            "probe: drive ended; {events_seen} events, log {}",
            log_path.display()
        );
        p.disconnect().await.ok();
        return Ok(());
    }

    let start = Instant::now();
    let (mut shots, mut events, mut statuses) = (Vec::new(), Vec::new(), 0u32);
    let mut kinds = [0u32; 3];
    let mut last_event_seq: Option<u32> = None;
    let mut gaps = 0u32;
    while start.elapsed() < Duration::from_secs(seconds) {
        let n = match tokio::time::timeout(Duration::from_millis(500), stream.next()).await {
            Ok(Some(n)) => n,
            Ok(None) => {
                println!("probe: the notification stream ended (disconnected?)");
                break;
            }
            Err(_) => continue,
        };
        if n.uuid == proto::SHOT_UUID {
            let s = proto::decode_shot(&n.value).ok_or("shot decode")?;
            println!(
                "probe: shot {} boot {:08X} kind {:?} x {:.2} y {:.2} event {} cal {:08X} sigma {:?} flags {:02X}",
                s.seq,
                s.boot,
                s.kind,
                s.x_mm(),
                s.y_mm(),
                s.event,
                s.cal,
                s.sigma_mm(),
                s.flags
            );
            shots.push(s);
        } else if n.uuid == proto::EVENT_UUID {
            let e = proto::decode_event(&n.value).ok_or("event decode")?;
            if let Some(last) = last_event_seq
                && e.seq != last + 1
            {
                gaps += 1;
                println!("probe: event gap: {last} -> {}", e.seq);
            }
            last_event_seq = Some(e.seq);
            kinds[e.kind as usize] += 1;
            if !cfg_seen.contains(&e.cfg) {
                // A change on the sensor: the cfg line for the event log.
                let (c, s) = read_config(&p).await?;
                let (id, line) = eventlog::cfg_line(&c, &s);
                println!("probe: new cfg {:08X} (read {id:08X})", e.cfg);
                cfg_seen.insert(id);
                log_lines.push(line);
            }
            log_lines.push(eventlog::event_line(&e));
            events.push(e);
        } else if n.uuid == proto::STATUS_UUID {
            statuses += 1;
        }
    }
    std::fs::write(&log_path, log_lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    let mut verdicts = std::collections::BTreeMap::new();
    for e in &events {
        *verdicts.entry(e.verdict_name().unwrap_or("-")).or_insert(0) += 1;
    }
    println!(
        "probe: SUMMARY {} shots ({} scored), {} events (acoustic {}, hole {}, missed {}), verdicts {:?}, {} event gaps, {} status notifications, log {}",
        shots.len(),
        shots.iter().filter(|s| s.scored()).count(),
        events.len(),
        kinds[0],
        kinds[1],
        kinds[2],
        verdicts,
        gaps,
        statuses,
        log_path.display()
    );
    // Every shot's event is one we got, with the sent flag.
    let linked = shots
        .iter()
        .filter(|s| s.event != 0)
        .filter(|s| {
            events
                .iter()
                .any(|e| e.seq == s.event && e.flags & proto::EV_SENT != 0)
        })
        .count();
    println!(
        "probe: shots linked to a received event marked sent: {linked} of {}",
        shots.iter().filter(|s| s.event != 0).count()
    );
    // The shots per boot, in the order they came: seq ranges and any gap (the shot store).
    let mut runs: Vec<(u32, u32, u32, u32)> = Vec::new(); // boot, first seq, last seq, gaps
    for s in &shots {
        match runs.last_mut() {
            Some(r) if r.0 == s.boot => {
                if s.seq != r.2 + 1 {
                    r.3 += 1;
                }
                r.2 = s.seq;
            }
            _ => runs.push((s.boot, s.seq, s.seq, 0)),
        }
    }
    for (boot, a, b, g) in &runs {
        println!("probe: shots of boot {boot:08X}: seq {a} to {b}, {g} gap(s)");
    }
    // PROBE_CLEAR=1: clear up to the last shot received, as the app's "Clear series".
    if std::env::var("PROBE_CLEAR").is_ok_and(|v| v == "1")
        && let Some(last) = shots.last()
    {
        p.write(&shot, &proto::encode_shot_clear(last.boot, last.seq), WriteType::WithResponse)
            .await
            .map_err(|e| e.to_string())?;
        println!("probe: clear sent: up to seq {} of boot {:08X}", last.seq, last.boot);
    }
    p.disconnect().await.ok();
    Ok(())
}

fn status_chr_ref(
    chars: &std::collections::BTreeSet<btleplug::api::Characteristic>,
) -> btleplug::api::Characteristic {
    chars
        .iter()
        .find(|c| c.uuid == proto::STATUS_UUID)
        .cloned()
        .unwrap()
}
