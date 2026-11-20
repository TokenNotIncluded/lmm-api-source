using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.IO;
using System.Runtime.Serialization.Json;
using System.Text;
using System.Xml;

// Only protocol framing lives here. JournalStorage.cs is the unchanged storage
// implementation, including its handle ownership and fail-closed ACL checks.
public static class LmmMagpieJournalProgram {
    const int MaximumInputBytes = 16384;
    const int MaximumOutputBytes = 4096;
    static readonly UTF8Encoding Utf8 = new UTF8Encoding(false, true);

    static void Need(bool condition) {
        if (!condition) throw new InvalidDataException("Invalid journal protocol.");
    }

    static bool JsonWhitespace(byte value) {
        return value == 0x20 || value == 0x09 || value == 0x0a || value == 0x0d;
    }

    static void RequireSingleObject(byte[] bytes, int length) {
        // The framework reader can report EOF after the first JSON value while
        // ignoring a second document. Only establish the outer byte boundary
        // here; the framework still validates JSON syntax, types and quotas.
        int start = 0;
        while (start < length && JsonWhitespace(bytes[start])) start++;
        Need(start < length && bytes[start] == (byte)'{');
        int depth = 0;
        bool quoted = false, escaped = false;
        for (int i = start; i < length; i++) {
            byte value = bytes[i];
            if (quoted) {
                if (escaped) escaped = false;
                else if (value == (byte)'\\') escaped = true;
                else if (value == (byte)'"') quoted = false;
            } else if (value == (byte)'"') quoted = true;
            else if (value == (byte)'{') depth++;
            else if (value == (byte)'}') {
                depth--;
                if (depth == 0) {
                    for (int rest = i + 1; rest < length; rest++) Need(JsonWhitespace(bytes[rest]));
                    return;
                }
            }
        }
        Need(false);
    }

    static Dictionary<string, string> ReadRequest() {
        // Read one extra byte to reject overflow. Reading through EOF, parsing,
        // storage and process exit all remain inside the parent's 5000 ms limit.
        byte[] bytes = new byte[MaximumInputBytes + 1];
        int length = 0;
        using (Stream input = Console.OpenStandardInput()) {
            while (length < bytes.Length) {
                int count = input.Read(bytes, length, bytes.Length - length);
                if (count == 0) break;
                length += count;
            }
        }
        Need(length > 0 && length <= MaximumInputBytes);
        Utf8.GetCharCount(bytes, 0, length); // Reject malformed UTF-8, not by UTF-16 length.
        RequireSingleObject(bytes, length);
        XmlDictionaryReaderQuotas quotas = new XmlDictionaryReaderQuotas {
            MaxDepth = 4,
            MaxStringContentLength = MaximumInputBytes,
            MaxArrayLength = 16,
            MaxBytesPerRead = MaximumOutputBytes,
            MaxNameTableCharCount = 512
        };
        Dictionary<string, string> values = new Dictionary<string, string>(StringComparer.Ordinal);
        using (XmlDictionaryReader reader = JsonReaderWriterFactory.CreateJsonReader(
            bytes, 0, length, Utf8, quotas, null)) {
            Need(reader.MoveToContent() == XmlNodeType.Element
                && reader.LocalName == "root" && reader.NamespaceURI == ""
                && reader.AttributeCount == 1 && reader.GetAttribute("type") == "object");
            // In the framework JSON mapping, an initial __type property becomes
            // another root attribute. The exact attribute count rejects it too.
            reader.ReadStartElement();
            while (reader.MoveToContent() == XmlNodeType.Element) {
                string name = reader.LocalName;
                Need(reader.NamespaceURI == "" && !values.ContainsKey(name)
                    && (name == "directory" || name == "name" || name == "action" || name == "content"));
                string type = reader.GetAttribute("type");
                Need(reader.AttributeCount == (type == null ? 0 : 1));
                if (name == "content" && type == "null") {
                    Need(reader.ReadElementContentAsString() == "");
                    values.Add(name, null);
                } else {
                    Need(type == null || type == "string");
                    values.Add(name, reader.ReadElementContentAsString());
                }
            }
            Need(reader.NodeType == XmlNodeType.EndElement && reader.LocalName == "root");
            reader.ReadEndElement();
            Need(reader.MoveToContent() == XmlNodeType.None && reader.EOF);
        }
        Need(values.Count == 4);
        return values;
    }

    static byte[] EncodeResult(LmmMagpieJournalResult result) {
        Need(result != null);
        using (MemoryStream output = new MemoryStream()) {
            using (XmlDictionaryWriter writer = JsonReaderWriterFactory.CreateJsonWriter(output, Utf8, false)) {
                writer.WriteStartElement("root");
                writer.WriteAttributeString("type", "object");
                writer.WriteStartElement("exists");
                writer.WriteAttributeString("type", "boolean");
                writer.WriteString(result.Exists ? "true" : "false");
                writer.WriteEndElement();
                writer.WriteStartElement("content");
                writer.WriteAttributeString("type", result.Content == null ? "null" : "string");
                if (result.Content != null) writer.WriteString(result.Content);
                writer.WriteEndElement();
                writer.WriteEndElement();
                writer.Flush();
            }
            Need(output.Length <= MaximumOutputBytes);
            return output.ToArray();
        }
    }

    static void WriteResponse(byte[] bytes) {
        Need(bytes.Length <= MaximumOutputBytes);
        using (Stream output = Console.OpenStandardOutput()) {
            output.Write(bytes, 0, bytes.Length);
            output.Flush();
        }
    }

    public static int Main(string[] args) {
        string stage = "input";
        try {
            Need(args.Length == 0);
            Dictionary<string, string> request = ReadRequest();
            stage = "storage";
            LmmMagpieJournalResult result = LmmMagpieJournalWindows.Run(
                request["directory"], request["name"], request["action"], request["content"]);
            stage = "result";
            WriteResponse(EncodeResult(result));
            return 0;
        } catch (Exception failure) {
            // Never emit exception text, paths, SIDs, input or record contents.
            int nativeCode = 0;
            Win32Exception native = failure.GetBaseException() as Win32Exception;
            if (native != null && native.NativeErrorCode >= 0 && native.NativeErrorCode <= 65535)
                nativeCode = native.NativeErrorCode;
            byte[] response = Utf8.GetBytes("{\"error\":\"unverified_windows_journal\",\"stage\":\""
                + stage + "\",\"nativeCode\":" + nativeCode.ToString(CultureInfo.InvariantCulture) + "}");
            try { WriteResponse(response); } catch { /* A closed output pipe is still a failure. */ }
            return 1;
        }
    }
}
