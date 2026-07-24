import XCTest
@testable import BuzzPushKit

final class PushWatermarkTests: XCTestCase {
    func testClampsFutureTimestampToAllowedClockSkew() {
        XCTAssertEqual(
            PushWatermark.persistedTimestamp(
                eventTimestamp: 2_000,
                now: 1_000,
                allowedFutureSkew: 300
            ),
            1_300
        )
    }

    func testPreservesTimestampWithinAllowedClockSkew() {
        XCTAssertEqual(
            PushWatermark.persistedTimestamp(
                eventTimestamp: 1_200,
                now: 1_000,
                allowedFutureSkew: 300
            ),
            1_200
        )
    }

    func testRejectsEventsBeyondAllowedClockSkew() {
        XCTAssertFalse(
            PushWatermark.isAcceptable(
                eventTimestamp: 1_301,
                now: 1_000,
                allowedFutureSkew: 300
            )
        )
        XCTAssertTrue(
            PushWatermark.isAcceptable(
                eventTimestamp: 1_300,
                now: 1_000,
                allowedFutureSkew: 300
            )
        )
    }

    func testRepairsPoisonedStoredWatermarkToCurrentTime() {
        XCTAssertEqual(
            PushWatermark.queryTimestamp(storedWatermark: 2_000, now: 1_000),
            1_000
        )
        XCTAssertEqual(
            PushWatermark.queryTimestamp(storedWatermark: 900, now: 1_000),
            900
        )
    }

    func testQuerySinceIsInclusiveForSameSecondEvents() {
        XCTAssertEqual(PushWatermark.querySince(watermark: 1_000), 1_000)
        XCTAssertNil(PushWatermark.querySince(watermark: 0))
    }

    func testFindsOnlyRemovedCommunityWatermarks() {
        XCTAssertEqual(
            PushWatermark.staleKeys(
                in: [
                    PushWatermark.key(communityID: "kept"),
                    PushWatermark.key(communityID: "removed"),
                    "unrelated",
                ],
                activeCommunityIDs: ["kept"]
            ),
            [PushWatermark.key(communityID: "removed")]
        )
    }
}
