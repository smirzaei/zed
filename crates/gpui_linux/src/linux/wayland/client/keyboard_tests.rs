use super::*;

#[test]
fn modifiers_before_keymap_leave_state_unchanged() {
    let (socket, _compositor_end) = UnixStream::pair().expect("socket pair");
    let conn = Connection::from_socket(socket).expect("connection over the socket pair");
    let backend = conn.backend().downgrade();
    let queue = conn.new_event_queue::<WaylandClientStatePtr>();
    let qh = queue.handle();
    let event_loop = calloop::EventLoop::<()>::try_new().expect("event loop");
    let (common, _, _) = LinuxCommon::new(event_loop.get_signal());
    let common = Rc::new(RefCell::new(common));
    let (frame_ping, _frame_ping_source) = calloop::ping::make_ping().expect("frame ping");
    let globals = Globals {
        qh: qh.clone(),
        activation: None,
        compositor: wl_compositor::WlCompositor::inert(backend.clone()),
        cursor_shape_manager: None,
        data_device_manager: None,
        primary_selection_manager: None,
        wm_base: xdg_wm_base::XdgWmBase::inert(backend.clone()),
        shm: wl_shm::WlShm::inert(backend.clone()),
        seat: wl_seat::WlSeat::inert(backend.clone()),
        viewporter: None,
        fractional_scale_manager: None,
        decoration_manager: None,
        layer_shell: None,
        kde_blur_manager: None,
        background_effects_manager: None,
        text_input_manager: None,
        gesture_manager: None,
        dialog: None,
        system_bell: None,
        executor: common.borrow().foreground_executor.clone(),
        frame_ping,
    };
    let cursor = Cursor::new(&conn, &globals, 24);
    let state = Rc::new(RefCell::new(WaylandClientState {
        serial_tracker: SerialTracker::new(),
        globals,
        gpu_context: Rc::new(RefCell::new(None)),
        compositor_gpu: None,
        wl_seat: wl_seat::WlSeat::inert(backend.clone()),
        wl_pointer: None,
        pinch_gesture: None,
        pinch_scale: 1.0,
        wl_keyboard: None,
        cursor_shape_device: None,
        data_device: None,
        primary_selection: None,
        text_input: None,
        pre_edit_text: None,
        ime_pre_edit: None,
        composing: false,
        last_ime_cursor_rectangle: None,
        windows: HashMap::default(),
        outputs: HashMap::default(),
        in_progress_outputs: HashMap::default(),
        wl_outputs: HashMap::default(),
        output_globals: HashMap::default(),
        keyboard_layout: LinuxKeyboardLayout::new(UNKNOWN_KEYBOARD_LAYOUT_NAME),
        keymap_state: None,
        compose_state: None,
        drag: DragState {
            data_offer: None,
            window: None,
            position: Point::default(),
            uri_read_generation: 0,
        },
        external_drag: None,
        click: ClickState {
            last_mouse_button: None,
            last_click: Instant::now(),
            last_location: Point::default(),
            current_count: 0,
        },
        repeat: KeyRepeat {
            characters_per_second: 16,
            delay: Duration::from_millis(500),
            current_id: 0,
            current_keycode: None,
        },
        modifiers: Modifiers::default(),
        capslock: Capslock::default(),
        axis_source: AxisSource::Wheel,
        mouse_location: None,
        continuous_scroll_delta: None,
        discrete_scroll_delta: None,
        vertical_modifier: -1.0,
        horizontal_modifier: -1.0,
        scroll_event_received: false,
        enter_token: None,
        button_pressed: None,
        mouse_focused_window: None,
        keyboard_focused_window: None,
        loop_handle: event_loop.handle(),
        cursor_style: None,
        cursor_hidden_window: None,
        clipboard: Clipboard::new(conn.clone(), event_loop.handle()),
        data_offers: Vec::new(),
        primary_data_offer: None,
        cursor,
        startup_activation_token: None,
        graphical_environment: GraphicalEnvironment::default(),
        registrations: Vec::new(),
        common,
        ime_enabled: None,
    }));
    let mut client = WaylandClientStatePtr(Rc::downgrade(&state));
    let keyboard = wl_keyboard::WlKeyboard::inert(backend);

    <WaylandClientStatePtr as Dispatch<wl_keyboard::WlKeyboard, ()>>::event(
        &mut client,
        &keyboard,
        wl_keyboard::Event::Modifiers {
            serial: 1,
            mods_depressed: 1,
            mods_latched: 0,
            mods_locked: 0,
            group: 0,
        },
        &(),
        &conn,
        &qh,
    );

    let state = state.borrow();
    assert_eq!(state.modifiers, Modifiers::default());
}
