package xyz.block.buzz.mobile;

import java.io.File;
import javax.xml.parsers.DocumentBuilderFactory;
import javax.xml.xpath.XPathFactory;
import org.junit.Test;
import static org.junit.Assert.assertEquals;

public class TranscriptBackupPolicyTest {
    @Test
    public void recordingsAreExcludedFromBackupAndDeviceTransfer() throws Exception {
        assertExcluded("backup_rules", "/full-backup-content");
        assertExcluded("data_extraction_rules", "/data-extraction-rules/cloud-backup");
        assertExcluded("data_extraction_rules", "/data-extraction-rules/device-transfer");
    }

    private void assertExcluded(String resource, String policy) throws Exception {
        File file = new File("src/main/res/xml", resource + ".xml");
        var document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file);
        var xpath = XPathFactory.newInstance().newXPath();
        assertEquals("1", xpath.evaluate(
            "count(" + policy + "/exclude[@domain='file' and @path='composer-transcripts/'])", document));
    }
}
