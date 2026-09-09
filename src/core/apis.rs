//! Google Advanced Services registry (clasp `core/apis.ts`): the list of
//! publicly available advanced services used to filter `list-apis` results
//! and to resolve enable/disable-api manifest entries. Data is transcribed
//! verbatim from clasp `PUBLIC_ADVANCED_SERVICES`.

use serde::Serialize;

/// An advanced service manifest entry (clasp `AdvancedService`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvancedService {
    /// The symbol used to access the service in the script (e.g. "Sheets").
    pub user_symbol: &'static str,
    /// The service version (e.g. "v4").
    pub version: &'static str,
    /// The service identifier (e.g. "sheets").
    pub service_id: &'static str,
}

/// All public advanced services (clasp `PUBLIC_ADVANCED_SERVICES`).
pub const PUBLIC_ADVANCED_SERVICES: [AdvancedService; 32] = [
    AdvancedService {
        user_symbol: "AdminDirectory",
        version: "directory_v1",
        service_id: "admin",
    },
    AdvancedService {
        user_symbol: "AdminGroupsMigration",
        version: "v1",
        service_id: "groupsmigration",
    },
    AdvancedService {
        user_symbol: "AdminGroupsSettings",
        version: "v1",
        service_id: "groupssettings",
    },
    AdvancedService {
        user_symbol: "AdminLicenseManager",
        version: "v1",
        service_id: "licensing",
    },
    AdvancedService {
        user_symbol: "AdminReports",
        version: "reports_v1",
        service_id: "admin",
    },
    AdvancedService {
        user_symbol: "AdminReseller",
        version: "v1",
        service_id: "reseller",
    },
    AdvancedService {
        user_symbol: "AdSense",
        version: "v2",
        service_id: "adsense",
    },
    AdvancedService {
        user_symbol: "Analytics",
        version: "v3",
        service_id: "analytics",
    },
    AdvancedService {
        user_symbol: "AnalyticsAdmin",
        version: "v1beta",
        service_id: "analyticsadmin",
    },
    AdvancedService {
        user_symbol: "AnalyticsData",
        version: "v1beta",
        service_id: "analyticsdata",
    },
    AdvancedService {
        user_symbol: "AnalyticsReporting",
        version: "v4",
        service_id: "analyticsreporting",
    },
    AdvancedService {
        user_symbol: "Area120Tables",
        version: "v1alpha1",
        service_id: "area120tables",
    },
    AdvancedService {
        user_symbol: "BigQuery",
        version: "v2",
        service_id: "bigquery",
    },
    AdvancedService {
        user_symbol: "Calendar",
        version: "v3",
        service_id: "calendar",
    },
    AdvancedService {
        user_symbol: "Chat",
        version: "v1",
        service_id: "chat",
    },
    AdvancedService {
        user_symbol: "Classroom",
        version: "v1",
        service_id: "classroom",
    },
    AdvancedService {
        user_symbol: "Docs",
        version: "v1",
        service_id: "docs",
    },
    AdvancedService {
        user_symbol: "DoubleClickCampaigns",
        version: "v4",
        service_id: "dfareporting",
    },
    AdvancedService {
        user_symbol: "Drive",
        version: "v3",
        service_id: "drive",
    },
    AdvancedService {
        user_symbol: "DriveActivity",
        version: "v2",
        service_id: "driveactivity",
    },
    AdvancedService {
        user_symbol: "DriveLabels",
        version: "v2beta",
        service_id: "drivelabels",
    },
    AdvancedService {
        user_symbol: "Gmail",
        version: "v1",
        service_id: "gmail",
    },
    AdvancedService {
        user_symbol: "People",
        version: "v1",
        service_id: "peopleapi",
    },
    AdvancedService {
        user_symbol: "Sheets",
        version: "v4",
        service_id: "sheets",
    },
    AdvancedService {
        user_symbol: "ShoppingContent",
        version: "v2.1",
        service_id: "content",
    },
    AdvancedService {
        user_symbol: "Slides",
        version: "v1",
        service_id: "slides",
    },
    AdvancedService {
        user_symbol: "TagManager",
        version: "v2",
        service_id: "tagmanager",
    },
    AdvancedService {
        user_symbol: "Tasks",
        version: "v1",
        service_id: "tasks",
    },
    AdvancedService {
        user_symbol: "WorkspaceEvents",
        version: "v1",
        service_id: "workspaceevents",
    },
    AdvancedService {
        user_symbol: "YouTube",
        version: "v3",
        service_id: "youtube",
    },
    AdvancedService {
        user_symbol: "YouTubeAnalytics",
        version: "v1",
        service_id: "youtubeAnalytics",
    },
    AdvancedService {
        user_symbol: "YouTubeContentId",
        version: "v1",
        service_id: "youtubePartner",
    },
];

impl AdvancedService {
    /// Finds a known advanced service by its `serviceId` (clasp
    /// `PUBLIC_ADVANCED_SERVICES.find(service => service.serviceId === …)`).
    pub fn by_service_id(service_id: &str) -> Option<&'static AdvancedService> {
        PUBLIC_ADVANCED_SERVICES
            .iter()
            .find(|service| service.service_id == service_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_advanced_services_by_service_id() {
        let sheets = AdvancedService::by_service_id("sheets").unwrap();
        assert_eq!(sheets.user_symbol, "Sheets");
        assert_eq!(sheets.version, "v4");
        assert!(AdvancedService::by_service_id("notaservice").is_none());
        // AdminDirectory and AdminReports share the 'admin' service id; the
        // first entry wins (clasp `find`).
        let admin = AdvancedService::by_service_id("admin").unwrap();
        assert_eq!(admin.user_symbol, "AdminDirectory");
    }
}
