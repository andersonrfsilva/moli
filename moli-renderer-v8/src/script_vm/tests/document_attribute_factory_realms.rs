use super::*;

#[test]
fn document_attribute_factory_realms() {
    let mut vm = new_parsed_test_vm(
        "https://factory-realms.test/",
        "<!doctype html><body></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const assert = (ok,label) => { if (!ok) throw Error(label); };
        const frame = document.body.appendChild(document.createElement('iframe'));
        const child = frame.contentWindow;

        for (const [owner,caller] of [[window,child],[child,window]]) {
            const doc = owner.document;
            const attribute = caller.Document.prototype.createAttribute.call(doc,'data-value');
            const ns = caller.Document.prototype.createAttributeNS.call(doc,'urn:test','p:value');
            for (const value of [attribute,ns]) {
                assert(Object.getPrototypeOf(value) === owner.Attr.prototype,'Attr owner prototype');
                assert(value.ownerDocument === doc,'Attr ownerDocument');
            }
            assert(ns.namespaceURI === 'urn:test' && ns.prefix === 'p','namespace data');
        }
        return true;
    })()"#).unwrap(), "true");
}
