using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.Serialization.Json;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Xml;

// Independent test oracle: translate the former PowerShell Framework ACL/SHA
// operations directly. No production storage implementation is called here.
public static class LmmWindowsTestAclProgram {
    const int MaximumInputBytes = 16384;
    const int MaximumOutputBytes = 4096;
    static readonly UTF8Encoding Utf8 = new UTF8Encoding(false, true);

    static void Need(bool condition) {
        if (!condition) throw new InvalidDataException("Invalid test ACL protocol.");
    }

    static bool Whitespace(byte value) {
        return value == 0x20 || value == 0x09 || value == 0x0a || value == 0x0d;
    }

    static void SingleObject(byte[] bytes, int length) {
        // The Framework JSON reader can ignore a second document. Enforce only
        // the outer boundary here; JSON grammar and types stay with the reader.
        int start = 0;
        while (start < length && Whitespace(bytes[start])) start++;
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
                    for (int rest = i + 1; rest < length; rest++) Need(Whitespace(bytes[rest]));
                    return;
                }
            }
        }
        Need(false);
    }

    static Dictionary<string, string> ReadRequest() {
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
        Utf8.GetCharCount(bytes, 0, length);
        SingleObject(bytes, length);
        XmlDictionaryReaderQuotas quotas = new XmlDictionaryReaderQuotas {
            MaxDepth = 4, MaxStringContentLength = MaximumInputBytes,
            MaxArrayLength = 4, MaxBytesPerRead = MaximumOutputBytes,
            MaxNameTableCharCount = 512
        };
        Dictionary<string, string> values = new Dictionary<string, string>(StringComparer.Ordinal);
        using (XmlDictionaryReader reader = JsonReaderWriterFactory.CreateJsonReader(
            bytes, 0, length, Utf8, quotas, null)) {
            Need(reader.MoveToContent() == XmlNodeType.Element
                && reader.LocalName == "root" && reader.NamespaceURI == ""
                && reader.AttributeCount == 1 && reader.GetAttribute("type") == "object");
            reader.ReadStartElement();
            while (reader.MoveToContent() == XmlNodeType.Element) {
                string name = reader.LocalName;
                Need(reader.NamespaceURI == "" && !values.ContainsKey(name)
                    && (name == "path" || name == "action"));
                string type = reader.GetAttribute("type");
                Need(reader.AttributeCount == (type == null ? 0 : 1));
                Need(type == null || type == "string");
                values.Add(name, reader.ReadElementContentAsString());
            }
            Need(reader.NodeType == XmlNodeType.EndElement && reader.LocalName == "root");
            reader.ReadEndElement();
            Need(reader.MoveToContent() == XmlNodeType.None && reader.EOF);
        }
        Need(values.Count == 2);
        return values;
    }

    static FileSystemSecurity ReadAcl(string path, bool directory) {
        return directory ? (FileSystemSecurity)Directory.GetAccessControl(path) : File.GetAccessControl(path);
    }

    static byte[] ObserveAcl(string path, string action) {
        FileAttributes attributes = File.GetAttributes(path);
        Need((attributes & FileAttributes.ReparsePoint) == 0);
        bool directory = (attributes & FileAttributes.Directory) != 0;
        SecurityIdentifier sid;
        using (WindowsIdentity identity = WindowsIdentity.GetCurrent()) {
            sid = identity.User;
            Need(sid != null);
        }
        FileSystemSecurity acl = ReadAcl(path, directory);
        if (action == "grant_other") {
            SecurityIdentifier other = new SecurityIdentifier("S-1-1-0");
            acl.AddAccessRule(new FileSystemAccessRule(other, FileSystemRights.Read, AccessControlType.Allow));
        } else if (action == "private") {
            acl = directory ? (FileSystemSecurity)new DirectorySecurity() : new FileSecurity();
            acl.SetOwner(sid);
            acl.SetAccessRuleProtection(true, false);
            InheritanceFlags inheritance = directory
                ? InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit
                : InheritanceFlags.None;
            acl.AddAccessRule(new FileSystemAccessRule(sid, FileSystemRights.FullControl,
                inheritance, PropagationFlags.None, AccessControlType.Allow));
        } else Need(action == "read");

        if (action != "read") {
            if (directory) Directory.SetAccessControl(path, (DirectorySecurity)acl);
            else File.SetAccessControl(path, (FileSecurity)acl);
        }
        acl = ReadAcl(path, directory);
        AuthorizationRuleCollection rules = acl.GetAccessRules(true, true, typeof(SecurityIdentifier));
        bool onlyCurrent = true;
        foreach (FileSystemAccessRule rule in rules) {
            if (rule.IdentityReference.Value != sid.Value
                || rule.AccessControlType != AccessControlType.Allow
                || rule.FileSystemRights != FileSystemRights.FullControl) onlyCurrent = false;
        }
        string fingerprint;
        using (SHA256 sha = SHA256.Create()) {
            byte[] descriptor = Utf8.GetBytes(acl.GetSecurityDescriptorSddlForm(
                AccessControlSections.Owner | AccessControlSections.Access));
            fingerprint = BitConverter.ToString(sha.ComputeHash(descriptor)).Replace("-", "").ToLowerInvariant();
        }
        bool ownerMatches = acl.GetOwner(typeof(SecurityIdentifier)).Value == sid.Value;
        return Utf8.GetBytes("{\"ownerMatches\":" + (ownerMatches ? "true" : "false")
            + ",\"protected\":" + (acl.AreAccessRulesProtected ? "true" : "false")
            + ",\"ruleCount\":" + rules.Count.ToString(CultureInfo.InvariantCulture)
            + ",\"onlyCurrent\":" + (onlyCurrent ? "true" : "false")
            + ",\"fingerprint\":\"" + fingerprint + "\"}");
    }

    public static int Main(string[] args) {
        string stage = "input";
        try {
            Need(args.Length == 0);
            Dictionary<string, string> request = ReadRequest();
            stage = "acl";
            byte[] result = ObserveAcl(request["path"], request["action"]);
            stage = "output";
            Need(result.Length <= MaximumOutputBytes);
            using (Stream output = Console.OpenStandardOutput()) {
                output.Write(result, 0, result.Length);
                output.Flush();
            }
            return 0;
        } catch {
            // No exception text, path, SID, input or descriptor is printed.
            try { Console.Error.WriteLine("windows_test_acl_" + stage + "_failed"); } catch { }
            return 1;
        }
    }
}
