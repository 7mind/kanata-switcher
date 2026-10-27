// Generated COSMIC protocols
pub(crate) mod cosmic_workspace {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/cosmic-workspace-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/protocols/cosmic-workspace-unstable-v1.xml");
}

pub(crate) mod cosmic_toplevel {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use super::super::cosmic_workspace::__interfaces::*;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/protocols/cosmic-toplevel-info-unstable-v1.xml");
    }
    use self::__interfaces::*;
    use super::cosmic_workspace::*;
    wayland_scanner::generate_client_code!("src/protocols/cosmic-toplevel-info-unstable-v1.xml");
}
